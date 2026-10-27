// SPDX-License-Identifier: AGPL-3.0-or-later
//! The MLS side of chat (RFC 9420, through OpenMLS): this device's identity, its key packages,
//! and each chat's group. Everything here runs on the device; SLAPorSLOP only relays the results.
//!
//! A device's credential names it as `slop:{user}:{device}`; its signing key is the one SLAPorSLOP
//! lists for that device, so every app can check who sent what.

use std::{collections::HashMap, sync::RwLock};

use openmls::prelude::{tls_codec::*, *};
use openmls_basic_credential::SignatureKeyPair;
use openmls_rust_crypto::{MemoryStorage, RustCrypto};
use openmls_traits::{OpenMlsProvider, crypto::OpenMlsCrypto, signatures::Signer};
use uuid::Uuid;

use crate::{Error, Result};

pub const CIPHERSUITE: Ciphersuite = Ciphersuite::MLS_128_DHKEMX25519_AES128GCM_SHA256_Ed25519;
/// Epochs back whose messages can still be read (a message sent just before a commit).
const PAST_EPOCHS: usize = 5;

fn mls_err(e: impl std::fmt::Display) -> Error {
    Error::Verify(format!("chat encryption: {e}"))
}

/// OpenMLS's crypto with storage we can save and load.
#[derive(Default)]
pub struct Provider {
    crypto: RustCrypto,
    storage: MemoryStorage,
}

impl OpenMlsProvider for Provider {
    type CryptoProvider = RustCrypto;
    type RandProvider = RustCrypto;
    type StorageProvider = MemoryStorage;
    fn storage(&self) -> &MemoryStorage {
        &self.storage
    }
    fn crypto(&self) -> &RustCrypto {
        &self.crypto
    }
    fn rand(&self) -> &RustCrypto {
        &self.crypto
    }
}

impl Provider {
    pub fn from_values(values: HashMap<Vec<u8>, Vec<u8>>) -> Self {
        Self {
            crypto: RustCrypto::default(),
            storage: MemoryStorage {
                values: RwLock::new(values),
            },
        }
    }

    pub fn values(&self) -> HashMap<Vec<u8>, Vec<u8>> {
        self.storage.values.read().unwrap().clone()
    }
}

/// A device's identity: its signing key and the credential naming it.
pub struct Identity {
    pub signer: SignatureKeyPair,
    pub credential: CredentialWithKey,
}

pub fn credential_name(user: Uuid, device: Uuid) -> Vec<u8> {
    format!("slop:{user}:{device}").into_bytes()
}

/// (user, device) from a credential, if it's one of SLAPorSLOP's.
pub fn parse_credential(credential: &Credential) -> Option<(Uuid, Uuid)> {
    let basic = BasicCredential::try_from(credential.clone()).ok()?;
    let text = std::str::from_utf8(basic.identity()).ok()?;
    let mut parts = text.strip_prefix("slop:")?.split(':');
    let user = parts.next()?.parse().ok()?;
    let device = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((user, device))
}

impl Identity {
    /// A new signing key, kept in the provider's storage. The credential is set once SLAPorSLOP has
    /// given the device its ID (`named`).
    pub fn generate(provider: &Provider) -> Result<SignatureKeyPair> {
        let signer = SignatureKeyPair::new(CIPHERSUITE.signature_algorithm()).map_err(mls_err)?;
        signer.store(provider.storage()).map_err(mls_err)?;
        Ok(signer)
    }

    pub fn load(provider: &Provider, public: &[u8], user: Uuid, device: Uuid) -> Result<Self> {
        let signer = SignatureKeyPair::read(
            provider.storage(),
            public,
            CIPHERSUITE.signature_algorithm(),
        )
        .ok_or_else(|| mls_err("this device's chat key is missing"))?;
        Ok(Self::named(signer, user, device))
    }

    pub fn named(signer: SignatureKeyPair, user: Uuid, device: Uuid) -> Self {
        let credential = CredentialWithKey {
            credential: BasicCredential::new(credential_name(user, device)).into(),
            signature_key: signer.to_public_vec().into(),
        };
        Self { signer, credential }
    }

    pub fn public_key(&self) -> Vec<u8> {
        self.signer.to_public_vec()
    }

    pub fn sign(&self, payload: &[u8]) -> Result<Vec<u8>> {
        self.signer
            .sign(payload)
            .map_err(|e| mls_err(format!("{e:?}")))
    }

    /// A key package for others to add this device with. The last-resort one can be reused.
    pub fn key_package(&self, provider: &Provider, last_resort: bool) -> Result<Vec<u8>> {
        let mut builder = KeyPackage::builder().leaf_node_capabilities(Capabilities::new(
            None,
            None,
            Some(&[ExtensionType::LastResort]),
            None,
            None,
        ));
        if last_resort {
            builder = builder.mark_as_last_resort();
        }
        let bundle = builder
            .build(CIPHERSUITE, provider, &self.signer, self.credential.clone())
            .map_err(mls_err)?;
        bundle
            .key_package()
            .tls_serialize_detached()
            .map_err(mls_err)
    }
}

/// Checks an Ed25519 signature (a device approval) with OpenMLS's crypto.
pub fn verify_signature(
    provider: &Provider,
    public: &[u8],
    payload: &[u8],
    signature: &[u8],
) -> bool {
    provider
        .crypto()
        .verify_signature(
            openmls_traits::types::SignatureScheme::ED25519,
            payload,
            public,
            signature,
        )
        .is_ok()
}

fn group_id(conversation: Uuid) -> GroupId {
    GroupId::from_slice(conversation.as_bytes())
}

pub fn create_group(provider: &Provider, me: &Identity, conversation: Uuid) -> Result<MlsGroup> {
    let config = MlsGroupCreateConfig::builder()
        .ciphersuite(CIPHERSUITE)
        .use_ratchet_tree_extension(true)
        .max_past_epochs(PAST_EPOCHS)
        .build();
    MlsGroup::new_with_group_id(
        provider,
        &me.signer,
        &config,
        group_id(conversation),
        me.credential.clone(),
    )
    .map_err(mls_err)
}

pub fn load_group(provider: &Provider, conversation: Uuid) -> Result<Option<MlsGroup>> {
    MlsGroup::load(provider.storage(), &group_id(conversation)).map_err(mls_err)
}

/// Forgets a chat's group (lost a race to create it, or removed from it).
pub fn delete_group(provider: &Provider, conversation: Uuid) -> Result<()> {
    if let Some(mut group) = load_group(provider, conversation)? {
        group.delete(provider.storage()).map_err(mls_err)?;
    }
    Ok(())
}

/// Joins a chat from a Welcome. Returns the group, or an error if the Welcome isn't for a key
/// package of this device.
pub fn join(provider: &Provider, welcome: &[u8]) -> Result<MlsGroup> {
    let message = MlsMessageIn::tls_deserialize_exact(welcome).map_err(mls_err)?;
    let MlsMessageBodyIn::Welcome(welcome) = message.extract() else {
        return Err(mls_err("not a Welcome"));
    };
    let config = MlsGroupJoinConfig::builder()
        .use_ratchet_tree_extension(true)
        .max_past_epochs(PAST_EPOCHS)
        .build();
    StagedWelcome::new_from_welcome(provider, &config, welcome, None)
        .map_err(mls_err)?
        .into_group(provider)
        .map_err(mls_err)
}

/// A group member: who and which device, with the key it signs with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupMember {
    pub index: LeafNodeIndex,
    pub user: Uuid,
    pub device: Uuid,
    pub signature_key: Vec<u8>,
}

pub fn members(group: &MlsGroup) -> Vec<GroupMember> {
    group
        .members()
        .filter_map(|m| {
            let (user, device) = parse_credential(&m.credential)?;
            Some(GroupMember {
                index: m.index,
                user,
                device,
                signature_key: m.signature_key.clone(),
            })
        })
        .collect()
}

/// A commit, and the Welcome for any devices it adds.
pub struct Commit {
    pub commit: Vec<u8>,
    pub welcome: Option<Vec<u8>>,
}

/// Adds and removes devices in one commit (left pending until SLAPorSLOP accepts it).
pub fn change_members(
    provider: &Provider,
    me: &Identity,
    group: &mut MlsGroup,
    add: &[Vec<u8>],
    remove: &[LeafNodeIndex],
) -> Result<Commit> {
    let mut packages = Vec::with_capacity(add.len());
    for kp in add {
        let parsed = KeyPackageIn::tls_deserialize_exact(kp.as_slice())
            .map_err(mls_err)?
            .validate(provider.crypto(), ProtocolVersion::Mls10)
            .map_err(mls_err)?;
        packages.push(parsed);
    }
    let bundle = group
        .commit_builder()
        .propose_removals(remove.iter().copied())
        .propose_adds(packages)
        .force_self_update(add.is_empty() && remove.is_empty())
        .load_psks(provider.storage())
        .map_err(mls_err)?
        .build(provider.rand(), provider.crypto(), &me.signer, |_| true)
        .map_err(mls_err)?
        .stage_commit(provider)
        .map_err(mls_err)?;
    let (commit, welcome, _) = bundle.into_contents();
    Ok(Commit {
        commit: commit.tls_serialize_detached().map_err(mls_err)?,
        welcome: welcome
            .map(|w| {
                MlsMessageOut::from_welcome(w, ProtocolVersion::Mls10)
                    .tls_serialize_detached()
                    .map_err(mls_err)
            })
            .transpose()?,
    })
}

pub fn merge_pending(provider: &Provider, group: &mut MlsGroup) -> Result<()> {
    group.merge_pending_commit(provider).map_err(mls_err)
}

pub fn clear_pending(provider: &Provider, group: &mut MlsGroup) -> Result<()> {
    group
        .clear_pending_commit(provider.storage())
        .map_err(mls_err)
}

pub fn encrypt(
    provider: &Provider,
    me: &Identity,
    group: &mut MlsGroup,
    plaintext: &[u8],
) -> Result<Vec<u8>> {
    group
        .create_message(provider, &me.signer, plaintext)
        .map_err(mls_err)?
        .tls_serialize_detached()
        .map_err(mls_err)
}

/// What an incoming message turned out to be.
pub enum Incoming {
    /// A message, from this device of this user.
    Application {
        user: Uuid,
        device: Uuid,
        plaintext: Vec<u8>,
    },
    /// A commit, already merged. `removed_me` when it took this device out of the chat.
    Commit { removed_me: bool },
}

pub fn process(provider: &Provider, group: &mut MlsGroup, message: &[u8]) -> Result<Incoming> {
    let message = MlsMessageIn::tls_deserialize_exact(message).map_err(mls_err)?;
    let protocol = message.try_into_protocol_message().map_err(mls_err)?;
    let processed = group.process_message(provider, protocol).map_err(mls_err)?;
    let sender = parse_credential(processed.credential());
    match processed.into_content() {
        ProcessedMessageContent::ApplicationMessage(m) => {
            let (user, device) =
                sender.ok_or_else(|| mls_err("a message from an unknown sender"))?;
            Ok(Incoming::Application {
                user,
                device,
                plaintext: m.into_bytes(),
            })
        }
        ProcessedMessageContent::StagedCommitMessage(commit) => {
            let removed_me = commit.self_removed();
            group
                .merge_staged_commit(provider, *commit)
                .map_err(mls_err)?;
            Ok(Incoming::Commit { removed_me })
        }
        // Proposals on their own aren't used; they arrive inside commits.
        _ => Ok(Incoming::Commit { removed_me: false }),
    }
}

pub fn epoch(group: &MlsGroup) -> i64 {
    group.epoch().as_u64() as i64
}
