//! Integration tests for /me endpoints.

use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use reqwest::StatusCode;
use serde::Deserialize;
use sqlx::PgPool;

use crate::helpers::TestFixture;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MeResponse {
    id: i64,
    email: String,
    name: Option<String>,
    github_username: Option<String>,
    created_at: String,
}

#[derive(Debug, Deserialize)]
struct OrganizationListResponse {
    organizations: Vec<OrganizationEntry>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct OrganizationEntry {
    id: i64,
    name: String,
    role: String,
    created_at: String,
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_me_returns_current_user(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/me")?;

    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let me = response.json::<MeResponse>().await?;
    pretty_assert_eq!(me.email, "alice@acme.com");
    pretty_assert_eq!(me.id, fixture.auth.account_id_alice().as_i64());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_me_requires_authentication(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/me")?;

    // No auth header
    let response = reqwest::Client::new().get(url.clone()).send().await?;
    pretty_assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Invalid session token
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth("invalid-session-token")
        .send()
        .await?;
    pretty_assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_me_organizations_returns_user_orgs(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/me/organizations")?;

    // Alice is admin of Acme Corp
    let response = reqwest::Client::new()
        .get(url.clone())
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let list = response.json::<OrganizationListResponse>().await?;
    pretty_assert_eq!(list.organizations.len(), 1);
    pretty_assert_eq!(list.organizations[0].name, "Acme Corp");
    pretty_assert_eq!(list.organizations[0].role, "admin");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_me_organizations_returns_correct_role(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/me/organizations")?;

    // Bob is a member (not admin) of Acme Corp
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let list = response.json::<OrganizationListResponse>().await?;
    pretty_assert_eq!(list.organizations.len(), 1);
    pretty_assert_eq!(list.organizations[0].name, "Acme Corp");
    pretty_assert_eq!(list.organizations[0].role, "member");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_me_organizations_requires_authentication(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/me/organizations")?;

    let response = reqwest::Client::new().get(url).send().await?;
    pretty_assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_me_with_different_users(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/me")?;

    // Test Charlie
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let me = response.json::<MeResponse>().await?;
    pretty_assert_eq!(me.email, "charlie@widget.com");
    pretty_assert_eq!(me.id, fixture.auth.account_id_charlie().as_i64());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_me_organizations_different_org(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/me/organizations")?;

    // Charlie is admin of Widget Inc
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let list = response.json::<OrganizationListResponse>().await?;
    pretty_assert_eq!(list.organizations.len(), 1);
    pretty_assert_eq!(list.organizations[0].name, "Widget Inc");
    pretty_assert_eq!(list.organizations[0].role, "admin");

    Ok(())
}
