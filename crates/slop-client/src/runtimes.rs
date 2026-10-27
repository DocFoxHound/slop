// SPDX-License-Identifier: AGPL-3.0-or-later
//! Installing shared runtimes (slop-manifest runtimes.rs) before a game that needs them starts.
//! Windows only: elsewhere every runtime counts as present.
//!
//! The file comes from the vendor's own link and is only run when Windows says it's validly
//! signed (Authenticode) by the vendor named in SLAPorSLOP's list. Its installer asks for admin
//! rights itself.

use std::path::Path;

#[cfg(windows)]
use slop_manifest::runtimes::Detect;
use slop_manifest::runtimes::Runtime;

use crate::{Error, Result};

/// Biggest runtime installer SLAPorSLOP downloads.
#[cfg(windows)]
const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Whether the runtime is already on this computer.
#[cfg(windows)]
pub fn installed(r: &Runtime) -> bool {
    match r.detect {
        Detect::Registry { key, value } => std::process::Command::new("reg")
            .args(["query", &format!(r"HKLM\{key}"), "/v", value])
            .output()
            .is_ok_and(|o| {
                o.status.success() && String::from_utf8_lossy(&o.stdout).contains("0x1")
            }),
        Detect::Versions { dir, prefix } => std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(prefix)),
        Detect::File { path } => Path::new(path).exists(),
    }
}

#[cfg(not(windows))]
pub fn installed(_: &Runtime) -> bool {
    true
}

/// Whether Windows says `file` is validly signed by `organisation` (Authenticode).
#[cfg(windows)]
fn signed_by(file: &Path, organisation: &str) -> Result<bool> {
    // The path goes in through the environment, never into the command text.
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$s = Get-AuthenticodeSignature -LiteralPath $env:SLOP_FILE; \
             [string]$s.Status + '|' + $s.SignerCertificate.Subject",
        ])
        .env("SLOP_FILE", file)
        .output()?;
    Ok(signature_ok(
        &String::from_utf8_lossy(&out.stdout),
        organisation,
    ))
}

/// `Valid|CN=…, O=Microsoft Corporation, …` names the organisation exactly.
pub fn signature_ok(answer: &str, organisation: &str) -> bool {
    let Some((status, subject)) = answer.trim().split_once('|') else {
        return false;
    };
    status == "Valid"
        && subject
            .split(", ")
            .any(|part| part.trim().trim_matches('"') == format!("O={organisation}"))
}

/// Downloads the runtime from its vendor, checks the signature, and runs its installer.
#[cfg(windows)]
pub async fn install(http: &reqwest::Client, r: &Runtime, staging: &Path) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    // A fresh folder of its own: the installer runs elevated, and Windows loads DLLs from the
    // installer's folder, so nothing else may be beside it.
    let dir = staging.join(format!("runtime-{}", crate::random_token()));
    std::fs::create_dir_all(staging)?;
    std::fs::create_dir(&dir)?;
    let file = dir.join(format!("{}.exe", r.id));
    let mut res = http
        .get(r.url)
        .send()
        .await
        .map_err(|e| Error::Install(format!("couldn't download {}: {e}", r.name)))?;
    if !res.status().is_success() {
        return Err(Error::Install(format!(
            "{}'s download answered {}",
            r.vendor,
            res.status()
        )));
    }
    let mut out = tokio::fs::File::create(&file).await?;
    let mut size = 0u64;
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|e| Error::Install(format!("the download of {} stopped: {e}", r.name)))?
    {
        size += chunk.len() as u64;
        if size > MAX_BYTES {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(Error::Install(format!(
                "{} is bigger than expected",
                r.name
            )));
        }
        out.write_all(&chunk).await?;
    }
    out.flush().await?;
    drop(out);
    let ok = signed_by(&file, r.signer).unwrap_or(false);
    if !ok {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(Error::Verify(format!(
            "the {} download isn't signed by {}, so SLAPorSLOP didn't run it",
            r.name, r.signer
        )));
    }
    let status = tokio::process::Command::new(&file)
        .args(r.args)
        .status()
        .await?;
    let _ = std::fs::remove_dir_all(&dir);
    // 3010 and 1641: installed, restart needed; 1638: a newer version is already there.
    match status.code() {
        Some(0 | 3010 | 1641 | 1638) => Ok(()),
        code => Err(Error::Install(format!(
            "{}'s installer stopped ({})",
            r.name,
            code.map_or("cancelled".into(), |c| c.to_string())
        ))),
    }
}

#[cfg(not(windows))]
pub async fn install(_: &reqwest::Client, r: &Runtime, _: &Path) -> Result<()> {
    Err(Error::Install(format!("{} is for Windows", r.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_named_vendors_valid_signature_counts() {
        let ms = "Valid|CN=Microsoft Corporation, O=Microsoft Corporation, L=Redmond, S=Washington, C=US";
        assert!(signature_ok(ms, "Microsoft Corporation"));
        assert!(!signature_ok(
            &ms.replace("Valid", "HashMismatch"),
            "Microsoft Corporation"
        ));
        assert!(!signature_ok(
            "Valid|CN=Evil, O=Microsoft Corporation Ltd, C=US",
            "Microsoft Corporation"
        ));
        assert!(!signature_ok("NotSigned|", "Microsoft Corporation"));
        assert!(!signature_ok("", "Microsoft Corporation"));
    }
}
