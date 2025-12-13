//! Integration tests for invitation endpoints.

use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

use crate::helpers::TestFixture;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CreateInvitationResponse {
    id: i64,
    token: String,
    role: String,
    expires_at: Option<String>,
    max_uses: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct InvitationListResponse {
    invitations: Vec<InvitationEntry>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct InvitationEntry {
    id: i64,
    role: String,
    created_at: String,
    expires_at: Option<String>,
    max_uses: Option<i32>,
    use_count: i32,
    revoked: bool,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct InvitationPreviewResponse {
    organization_name: String,
    role: String,
    expires_at: Option<String>,
    valid: bool,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AcceptInvitationResponse {
    organization_id: i64,
    organization_name: String,
    role: String,
}

#[derive(Debug, Serialize)]
struct CreateInvitationRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_uses: Option<i32>,
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_invitation_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;

    let expires_at = (OffsetDateTime::now_utc() + Duration::days(7)).format(&Rfc3339)?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&CreateInvitationRequest {
            role: Some(String::from("member")),
            expires_at: Some(expires_at),
            max_uses: Some(5),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::CREATED);

    let inv = response.json::<CreateInvitationResponse>().await?;
    pretty_assert_eq!(inv.role, "member");
    pretty_assert_eq!(inv.max_uses, Some(5));
    assert!(!inv.token.is_empty());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_invitation_default_values(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;

    // Send empty request - should use defaults (never expires)
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::CREATED);

    let inv = response.json::<CreateInvitationResponse>().await?;
    pretty_assert_eq!(inv.role, "member");
    pretty_assert_eq!(inv.expires_at, None); // Never expires by default
    pretty_assert_eq!(inv.max_uses, None); // Unlimited by default

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_invitation_non_admin_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;

    // Bob is a member, not an admin
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_invitation_non_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;

    // Charlie is not a member of Acme
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_invitations_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create an invitation first
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;

    // List invitations
    let list_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let response = reqwest::Client::new()
        .get(list_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let list = response.json::<InvitationListResponse>().await?;
    pretty_assert_eq!(list.invitations.len(), 1);
    pretty_assert_eq!(list.invitations[0].use_count, 0);
    assert!(!list.invitations[0].revoked);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_invitations_non_admin_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;

    // Bob is a member, not an admin
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn revoke_invitation_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create an invitation first
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;
    let inv = create_response.json::<CreateInvitationResponse>().await?;

    // Revoke the invitation
    let revoke_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/invitations/{}",
        inv.id
    ))?;
    let response = reqwest::Client::new()
        .delete(revoke_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify it's revoked by listing
    let list_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let list_response = reqwest::Client::new()
        .get(list_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    let list = list_response.json::<InvitationListResponse>().await?;
    assert!(list.invitations[0].revoked);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn revoke_invitation_non_admin_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create an invitation first
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;
    let inv = create_response.json::<CreateInvitationResponse>().await?;

    // Bob tries to revoke
    let revoke_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/invitations/{}",
        inv.id
    ))?;
    let response = reqwest::Client::new()
        .delete(revoke_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_invitation_preview_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create an invitation
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&CreateInvitationRequest {
            role: Some(String::from("admin")),
            expires_at: None,
            max_uses: None,
        })
        .send()
        .await?;
    let inv = create_response.json::<CreateInvitationResponse>().await?;

    // Get preview (no auth required)
    let preview_url = fixture
        .base_url
        .join(&format!("api/v1/invitations/{}", inv.token))?;
    let response = reqwest::Client::new().get(preview_url).send().await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let preview = response.json::<InvitationPreviewResponse>().await?;
    pretty_assert_eq!(preview.organization_name, "Acme Corp");
    pretty_assert_eq!(preview.role, "admin");
    assert!(preview.valid);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn get_invitation_preview_not_found(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;

    let preview_url = fixture
        .base_url
        .join("api/v1/invitations/nonexistent-token")?;
    let response = reqwest::Client::new().get(preview_url).send().await?;

    pretty_assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn accept_invitation_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create an invitation
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;
    let inv = create_response.json::<CreateInvitationResponse>().await?;

    // Charlie accepts the invitation (he's not a member of Acme)
    let accept_url = fixture
        .base_url
        .join(&format!("api/v1/invitations/{}/accept", inv.token))?;
    let response = reqwest::Client::new()
        .post(accept_url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let result = response.json::<AcceptInvitationResponse>().await?;
    pretty_assert_eq!(result.organization_name, "Acme Corp");
    pretty_assert_eq!(result.role, "member");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn accept_invitation_already_member_conflict(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create an invitation
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;
    let inv = create_response.json::<CreateInvitationResponse>().await?;

    // Bob tries to accept (he's already a member)
    let accept_url = fixture
        .base_url
        .join(&format!("api/v1/invitations/{}/accept", inv.token))?;
    let response = reqwest::Client::new()
        .post(accept_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::CONFLICT);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn accept_invitation_revoked_bad_request(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create and revoke an invitation
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;
    let inv = create_response.json::<CreateInvitationResponse>().await?;

    // Revoke it
    let revoke_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/invitations/{}",
        inv.id
    ))?;
    reqwest::Client::new()
        .delete(revoke_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    // Charlie tries to accept the revoked invitation
    let accept_url = fixture
        .base_url
        .join(&format!("api/v1/invitations/{}/accept", inv.token))?;
    let response = reqwest::Client::new()
        .post(accept_url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn accept_invitation_requires_auth(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create an invitation
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/invitations"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({}))
        .send()
        .await?;
    let inv = create_response.json::<CreateInvitationResponse>().await?;

    // Try to accept without auth
    let accept_url = fixture
        .base_url
        .join(&format!("api/v1/invitations/{}/accept", inv.token))?;
    let response = reqwest::Client::new().post(accept_url).send().await?;

    pretty_assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}
