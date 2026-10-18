//! What a viewer may see, from the per-country policy table (LEGAL-GUIDANCE AGE-1, AGE-4, AGE-5).

use axum::http::HeaderMap;
use sqlx::PgPool;

use crate::{auth::session::CurrentUser, config::Config, error::ApiResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewerPolicy {
    /// Apps with a higher `min_age` are hidden.
    pub max_min_age: i16,
}

impl ViewerPolicy {
    /// The UK under-18 view: 18+ content hidden. Also the fail-safe when the country is unknown.
    pub const UNDER_18_VIEW: Self = Self { max_min_age: 16 };
    pub const FULL_VIEW: Self = Self { max_min_age: 18 };

    /// Signed-in viewers use their account's country. Signed-out viewers use the country header
    /// from the CDN; with no header or an unknown country they get the under-18 view.
    pub async fn resolve(
        db: &PgPool,
        config: &Config,
        user: Option<&CurrentUser>,
        headers: &HeaderMap,
    ) -> ApiResult<Self> {
        let country = match user {
            Some(user) => Some(user.country_code.clone()),
            None => config
                .country_header
                .as_deref()
                .and_then(|name| headers.get(name))
                .and_then(|v| v.to_str().ok())
                .map(|v| v.trim().to_ascii_uppercase()),
        };
        let Some(country) = country else {
            return Ok(Self::UNDER_18_VIEW);
        };

        // No age check exists yet (AGE-6), so nothing lifts an under-18 view default.
        let under18_default: Option<bool> = sqlx::query_scalar(
            "select under18_view_default from country_policies where country_code = $1",
        )
        .bind(&country)
        .fetch_optional(db)
        .await?;

        Ok(match under18_default {
            Some(false) => Self::FULL_VIEW,
            _ => Self::UNDER_18_VIEW,
        })
    }
}
