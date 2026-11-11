// SPDX-License-Identifier: AGPL-3.0-or-later
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
        Self::for_country(db, country.as_deref()).await
    }

    /// The view in this country; with none, the under-18 view.
    async fn for_country(db: &PgPool, country: Option<&str>) -> ApiResult<Self> {
        let Some(country) = country else {
            return Ok(Self::UNDER_18_VIEW);
        };

        // No age check exists yet (AGE-6), so nothing lifts an under-18 view default.
        let under18_default: Option<bool> = sqlx::query_scalar(
            "select under18_view_default from country_policies where country_code = $1",
        )
        .bind(country)
        .fetch_optional(db)
        .await?;

        Ok(match under18_default {
            Some(false) => Self::FULL_VIEW,
            _ => Self::UNDER_18_VIEW,
        })
    }
}

/// SQL: the highest `min_age` the account in `users` row `{u}` may see. The SQL form of
/// `resolve` for a signed-in account; the ages come from the constants above, nowhere else.
/// `u` is always an alias written in the code, never input.
pub(crate) fn account_max_min_age_sql(u: &str) -> String {
    format!(
        "(case when (select cp.under18_view_default from country_policies cp
                     where cp.country_code = {u}.country_code) is false then {} else {} end)",
        ViewerPolicy::FULL_VIEW.max_min_age,
        ViewerPolicy::UNDER_18_VIEW.max_min_age
    )
}

#[cfg(test)]
mod tests {
    use sqlx::PgPool;

    use super::{ViewerPolicy, account_max_min_age_sql};

    /// A full-view country, an under-18-view one, and one with no policy row.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn the_sql_form_agrees_with_resolve(db: PgPool) {
        for (country, view) in [
            ("US", ViewerPolicy::FULL_VIEW),
            ("GB", ViewerPolicy::UNDER_18_VIEW),
            ("ZZ", ViewerPolicy::UNDER_18_VIEW),
        ] {
            let sql: i32 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "select {} from (select $1::text as country_code) u",
                account_max_min_age_sql("u")
            )))
            .bind(country)
            .fetch_one(&db)
            .await
            .unwrap();
            let resolved = ViewerPolicy::for_country(&db, Some(country)).await.unwrap();
            assert_eq!(resolved, view, "{country}");
            assert_eq!(sql, i32::from(resolved.max_min_age), "{country}");
        }
    }
}
