//! Integration tests for organization management endpoints.

use color_eyre::Result;
use pretty_assertions::assert_eq as pretty_assert_eq;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::helpers::TestFixture;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CreateOrgResponse {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct MemberListResponse {
    members: Vec<MemberEntry>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct MemberEntry {
    account_id: i64,
    email: String,
    name: Option<String>,
    role: String,
    joined_at: String,
}

#[derive(Debug, Serialize)]
struct UpdateRoleRequest {
    role: String,
}

// =============================================================================
// Create Organization Tests
// =============================================================================

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_organization_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/organizations")?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({ "name": "New Org" }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::CREATED);

    let org = response.json::<CreateOrgResponse>().await?;
    pretty_assert_eq!(org.name, "New Org");
    assert!(org.id > 0);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_organization_empty_name_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/organizations")?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({ "name": "  " }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_organization_requires_auth(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let url = fixture.base_url.join("api/v1/organizations")?;

    let response = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({ "name": "New Org" }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}

// =============================================================================
// List Members Tests
// =============================================================================

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_members_as_admin(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members"))?;

    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    let list = response.json::<MemberListResponse>().await?;
    pretty_assert_eq!(list.members.len(), 2); // Alice and Bob

    let alice = list
        .members
        .iter()
        .find(|m| m.email == "alice@acme.com")
        .expect("Alice should be in the list");
    pretty_assert_eq!(alice.role, "admin");

    let bob = list
        .members
        .iter()
        .find(|m| m.email == "bob@acme.com")
        .expect("Bob should be in the list");
    pretty_assert_eq!(bob.role, "member");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_members_as_member(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members"))?;

    // Bob is a member (not admin) of Acme
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::OK);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_members_non_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members"))?;

    // Charlie is not a member of Acme
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

// =============================================================================
// Update Member Role Tests
// =============================================================================

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn update_member_role_promote_to_admin(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let bob_id = fixture.auth.account_id_bob().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{bob_id}"))?;

    // Alice (admin) promotes Bob to admin
    let response = reqwest::Client::new()
        .patch(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&UpdateRoleRequest {
            role: String::from("admin"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::NO_CONTENT);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn update_member_role_non_admin_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let alice_id = fixture.auth.account_id_alice().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{alice_id}"))?;

    // Bob (member) tries to demote Alice
    let response = reqwest::Client::new()
        .patch(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .json(&UpdateRoleRequest {
            role: String::from("member"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn update_member_role_demote_last_admin_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let alice_id = fixture.auth.account_id_alice().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{alice_id}"))?;

    // Alice tries to demote herself (she's the only admin)
    let response = reqwest::Client::new()
        .patch(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&UpdateRoleRequest {
            role: String::from("member"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

// =============================================================================
// Remove Member Tests
// =============================================================================

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn remove_member_success(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let bob_id = fixture.auth.account_id_bob().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{bob_id}"))?;

    // Alice (admin) removes Bob
    let response = reqwest::Client::new()
        .delete(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Verify Bob is no longer a member
    let list_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members"))?;
    let list_response = reqwest::Client::new()
        .get(list_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    let list = list_response.json::<MemberListResponse>().await?;
    assert!(
        !list.members.iter().any(|m| m.email == "bob@acme.com"),
        "Bob should no longer be in the member list"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn remove_member_non_admin_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let alice_id = fixture.auth.account_id_alice().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{alice_id}"))?;

    // Bob (member) tries to remove Alice
    let response = reqwest::Client::new()
        .delete(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn remove_self_via_delete_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let alice_id = fixture.auth.account_id_alice().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{alice_id}"))?;

    // Alice tries to remove herself via DELETE
    let response = reqwest::Client::new()
        .delete(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

// =============================================================================
// Leave Organization Tests
// =============================================================================

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn leave_organization_as_member(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/leave"))?;

    // Bob (member) leaves
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::NO_CONTENT);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn leave_organization_last_admin_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/leave"))?;

    // Alice (only admin) tries to leave
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn leave_organization_not_member(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/leave"))?;

    // Charlie is not a member of Acme
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::NOT_FOUND);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn leave_organization_admin_after_promoting_another(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let bob_id = fixture.auth.account_id_bob().as_i64();

    // First, promote Bob to admin
    let promote_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{bob_id}"))?;
    let promote_response = reqwest::Client::new()
        .patch(promote_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&UpdateRoleRequest {
            role: String::from("admin"),
        })
        .send()
        .await?;
    pretty_assert_eq!(promote_response.status(), StatusCode::NO_CONTENT);

    // Now Alice can leave
    let leave_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/leave"))?;
    let leave_response = reqwest::Client::new()
        .post(leave_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(leave_response.status(), StatusCode::NO_CONTENT);

    Ok(())
}
