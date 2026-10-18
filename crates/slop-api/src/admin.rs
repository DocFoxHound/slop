//! Review and moderation actions, run by Slop staff from the command line (`slop-api review …`).
//!
//! Phase 1 has no web admin: an operator with production access runs these, which keeps the
//! admin surface off the public site (SECURITY §8.4). Every action is written to the audit log.

use anyhow::{Context, bail, ensure};
use serde_json::json;
use slop_manifest::{
    FileKind, Launch, MANIFEST_SCHEMA, ManifestFile, Platform, Purpose, ReleaseManifest,
};
use sqlx::PgPool;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::catalog::AppStage;

async fn audit(
    tx: &mut sqlx::PgConnection,
    actor: &str,
    action: &str,
    subject: &str,
    detail: serde_json::Value,
) -> anyhow::Result<()> {
    sqlx::query("insert into audit_log (actor, action, subject, detail) values ($1, $2, $3, $4)")
        .bind(actor)
        .bind(action)
        .bind(subject)
        .bind(detail)
        .execute(&mut *tx)
        .await?;
    Ok(())
}

fn check_reviewer(reviewer: &str) -> anyhow::Result<()> {
    ensure!(
        !reviewer.trim().is_empty(),
        "say who you are with --reviewer"
    );
    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct PendingRelease {
    pub id: Uuid,
    pub app_slug: String,
    pub version: String,
    pub commit_sha: String,
    pub commit_verified: bool,
    pub repo_url: String,
    pub created_at: OffsetDateTime,
}

/// Releases waiting for a reviewer, oldest first.
pub async fn pending(db: &PgPool) -> anyhow::Result<Vec<PendingRelease>> {
    Ok(sqlx::query_as(
        "select r.id, a.slug as app_slug, r.version, r.commit_sha, r.commit_verified, a.repo_url, r.created_at
         from releases r join apps a on a.id = r.app_id
         where r.status = 'awaiting_review' order by r.created_at",
    )
    .fetch_all(db)
    .await?)
}

#[derive(sqlx::FromRow)]
struct ReleaseRow {
    status: String,
    app_id: Uuid,
    slug: String,
    title: String,
    maker: String,
    repo_url: String,
    version: String,
    commit_sha: String,
}

#[derive(sqlx::FromRow)]
struct FileRow {
    platform: String,
    kind: String,
    url: String,
    file_name: String,
    exec: Option<String>,
    args: Vec<String>,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    attestation: Option<String>,
    provenance_problem: Option<String>,
}

/// Approves a release and signs its manifest. With `require_maker_signature`, every file must
/// have a verified maker attestation (SECURITY §5.3).
pub async fn approve(
    db: &PgPool,
    key: Option<&slop_manifest::ed25519_dalek::SigningKey>,
    require_maker_signature: bool,
    release_id: Uuid,
    reviewer: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let key = key.context("SLOP_SIGNING_KEY isn't set, so nothing can be signed")?;
    let mut tx = db.begin().await?;
    let release: ReleaseRow = sqlx::query_as(
        "select r.status::text as status, a.id as app_id, a.slug, a.title, u.display_name as maker,
                a.repo_url, r.version, r.commit_sha
         from releases r join apps a on a.id = r.app_id join users u on u.id = a.maker_id
         where r.id = $1 for update of r",
    )
    .bind(release_id)
    .fetch_optional(&mut *tx)
    .await?
    .context("no such release")?;
    let status = &release.status;
    ensure!(
        status == "awaiting_review",
        "release is {status}, not awaiting review"
    );

    let rows: Vec<FileRow> = sqlx::query_as(
        "select platform, kind, url, file_name, exec, args, size_bytes, sha256, attestation, provenance_problem
         from release_files where release_id = $1 order by platform",
    )
    .bind(release_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut files = Vec::new();
    for f in rows {
        let (Some(size), Some(sha256)) = (f.size_bytes, f.sha256) else {
            bail!("the {} build hasn't been hashed", f.platform);
        };
        if require_maker_signature && f.attestation.is_none() {
            bail!(
                "the {} build has no verified maker signature: {}",
                f.platform,
                f.provenance_problem.as_deref().unwrap_or("not checked")
            );
        }
        files.push(ManifestFile {
            platform: Platform::parse(&f.platform).context("bad platform")?,
            kind: FileKind::parse(&f.kind).context("bad kind")?,
            url: f.url,
            file_name: f.file_name,
            size: size as u64,
            sha256,
            launch: f.exec.map(|exec| Launch { exec, args: f.args }),
            attestation: f.attestation,
        });
    }
    ensure!(!files.is_empty(), "release has no files");

    let now = OffsetDateTime::now_utc().replace_nanosecond(0)?;
    let manifest = ReleaseManifest {
        schema: MANIFEST_SCHEMA,
        app_id: release.app_id.to_string(),
        app_slug: release.slug.clone(),
        app_title: release.title,
        maker: release.maker,
        repo_url: release.repo_url,
        release_id: release_id.to_string(),
        version: release.version.clone(),
        commit: release.commit_sha,
        files,
        approved_at: now.format(&Rfc3339)?,
    };
    let signed = slop_manifest::sign(&manifest, Purpose::ReleaseManifest, key);
    sqlx::query(
        "update releases set status = 'approved', reviewed_at = now(), signed_manifest = $2, status_reason = null
         where id = $1",
    )
    .bind(release_id)
    .bind(sqlx::types::Json(&signed))
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        reviewer,
        "release.approve",
        &release_id.to_string(),
        json!({ "app": release.slug, "version": release.version, "key_id": signed.key_id }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Rejects a release. The reason is shown to the maker.
pub async fn reject(
    db: &PgPool,
    release_id: Uuid,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    ensure!(
        !reason.trim().is_empty(),
        "give the maker a reason with --reason"
    );
    let mut tx = db.begin().await?;
    let updated = sqlx::query(
        "update releases set status = 'rejected', status_reason = $2, reviewed_at = now()
         where id = $1 and status in ('awaiting_review', 'fetching', 'failed')",
    )
    .bind(release_id)
    .bind(reason.trim())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    ensure!(updated == 1, "no release waiting for review with that ID");
    audit(
        &mut tx,
        reviewer,
        "release.reject",
        &release_id.to_string(),
        json!({ "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The kill switch (SECURITY §5.5): a revoked release stops launching on every client.
pub async fn revoke(
    db: &PgPool,
    release_id: Uuid,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    ensure!(
        !reason.trim().is_empty(),
        "say why with --reason; it's shown to players"
    );
    let mut tx = db.begin().await?;
    let updated = sqlx::query(
        "update releases set status = 'revoked', status_reason = $2, revoked_at = now()
         where id = $1 and status = 'approved'",
    )
    .bind(release_id)
    .bind(reason.trim())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    ensure!(updated == 1, "no approved release with that ID");
    audit(
        &mut tx,
        reviewer,
        "release.revoke",
        &release_id.to_string(),
        json!({ "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn has_approved_release(tx: &mut sqlx::PgConnection, app_id: Uuid) -> anyhow::Result<bool> {
    Ok(sqlx::query_scalar(
        "select exists (select 1 from releases where app_id = $1 and status = 'approved')",
    )
    .bind(app_id)
    .fetch_one(&mut *tx)
    .await?)
}

/// Puts a reviewed draft in the store. Beta and Released apps need an approved release first;
/// Coming soon pages have no build (DESIGN §8).
pub async fn list_app(db: &PgPool, slug: &str, reviewer: &str) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, AppStage, String)> =
        sqlx::query_as("select id, stage, status::text from apps where slug = $1 for update")
            .bind(slug)
            .fetch_optional(&mut *tx)
            .await?;
    let (id, stage, status) = row.context("no such app")?;
    ensure!(status != "listed", "already listed");
    if stage != AppStage::ComingSoon {
        ensure!(
            has_approved_release(&mut tx, id).await?,
            "approve a release before listing a {stage:?} app"
        );
    }
    sqlx::query("update apps set status = 'listed', updated_at = now() where id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit(
        &mut tx,
        reviewer,
        "app.list",
        slug,
        json!({ "from": status }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Takes an app out of the store (moderation). Installs keep working unless a release is revoked.
pub async fn delist_app(
    db: &PgPool,
    slug: &str,
    reviewer: &str,
    reason: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    ensure!(!reason.trim().is_empty(), "say why with --reason");
    let mut tx = db.begin().await?;
    let updated = sqlx::query("update apps set status = 'delisted', updated_at = now() where slug = $1 and status = 'listed'")
        .bind(slug)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    ensure!(updated == 1, "no listed app with that address");
    audit(
        &mut tx,
        reviewer,
        "app.delist",
        slug,
        json!({ "reason": reason }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Moves an app to a new stage. Beta and Released need an approved release.
pub async fn set_stage(
    db: &PgPool,
    slug: &str,
    stage: AppStage,
    reviewer: &str,
) -> anyhow::Result<()> {
    check_reviewer(reviewer)?;
    let mut tx = db.begin().await?;
    let row: Option<(Uuid, AppStage)> =
        sqlx::query_as("select id, stage from apps where slug = $1 for update")
            .bind(slug)
            .fetch_optional(&mut *tx)
            .await?;
    let (id, from) = row.context("no such app")?;
    ensure!(from != stage, "already in that stage");
    if stage != AppStage::ComingSoon {
        ensure!(
            has_approved_release(&mut tx, id).await?,
            "approve a release first"
        );
    }
    sqlx::query(
        "update apps set stage = $2, stage_entered_at = now(), updated_at = now() where id = $1",
    )
    .bind(id)
    .bind(stage)
    .execute(&mut *tx)
    .await?;
    audit(
        &mut tx,
        reviewer,
        "app.stage",
        slug,
        json!({ "from": from, "to": stage }),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
