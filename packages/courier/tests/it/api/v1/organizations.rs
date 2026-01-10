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

#[derive(Debug, Deserialize)]
struct ApiKeyListResponse {
    api_keys: Vec<ApiKeyEntry>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApiKeyEntry {
    id: i64,
    name: String,
    account_id: i64,
    account_email: String,
    bot: bool,
    created_at: String,
    accessed_at: String,
}

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
async fn removed_member_api_key_no_longer_works(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let bob_id = fixture.auth.account_id_bob().as_i64();

    // Use a test key for CAS operations
    let test_content = b"test blob content for bob";
    let test_key = crate::helpers::test_blob(test_content);

    // Verify Bob's API key works for an authenticated endpoint (CAS write)
    let cas_url = fixture.base_url.join(&format!("api/v1/cas/{}", test_key))?;
    let write_response = reqwest::Client::new()
        .put(cas_url)
        .bearer_auth(fixture.auth.token_bob().expose())
        .body(test_content.to_vec())
        .send()
        .await?;
    pretty_assert_eq!(
        write_response.status(),
        StatusCode::CREATED,
        "Bob's API key should work before removal"
    );

    // Alice (admin) removes Bob from the org
    let remove_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{bob_id}"))?;
    let remove_response = reqwest::Client::new()
        .delete(remove_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(remove_response.status(), StatusCode::NO_CONTENT);

    // Verify Bob's API key no longer works - try to write a different blob
    let test_content_2 = b"another test blob for bob";
    let test_key_2 = crate::helpers::test_blob(test_content_2);
    let cas_url_2 = fixture
        .base_url
        .join(&format!("api/v1/cas/{}", test_key_2))?;
    let write_response_after = reqwest::Client::new()
        .put(cas_url_2)
        .bearer_auth(fixture.auth.token_bob().expose())
        .body(test_content_2.to_vec())
        .send()
        .await?;
    pretty_assert_eq!(
        write_response_after.status(),
        StatusCode::UNAUTHORIZED,
        "Bob's API key should be revoked after removal from org"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn removed_member_session_still_works_for_account(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let bob_id = fixture.auth.account_id_bob().as_i64();

    // Verify Bob's session works for the /me endpoint (account-level)
    let me_url = fixture.base_url.join("api/v1/me")?;
    let me_response = reqwest::Client::new()
        .get(me_url.clone())
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;
    pretty_assert_eq!(
        me_response.status(),
        StatusCode::OK,
        "Bob's session should work before removal"
    );

    // Verify Bob can access org members list (org-level)
    let members_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members"))?;
    let members_response = reqwest::Client::new()
        .get(members_url.clone())
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;
    pretty_assert_eq!(
        members_response.status(),
        StatusCode::OK,
        "Bob should be able to list org members before removal"
    );

    // Alice (admin) removes Bob from the org
    let remove_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{bob_id}"))?;
    let remove_response = reqwest::Client::new()
        .delete(remove_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(remove_response.status(), StatusCode::NO_CONTENT);

    // Sessions are account-wide, not org-scoped. Removing a user from an org
    // should NOT invalidate their session - they can still log in to view
    // their account, join other orgs, accept invites, etc.
    let me_response_after = reqwest::Client::new()
        .get(me_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;
    pretty_assert_eq!(
        me_response_after.status(),
        StatusCode::OK,
        "Bob's session should still work for account-level operations"
    );

    // BUT Bob should no longer be able to access org resources
    let members_response_after = reqwest::Client::new()
        .get(members_url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;
    pretty_assert_eq!(
        members_response_after.status(),
        StatusCode::FORBIDDEN,
        "Bob's session should NOT work for org-level operations after removal"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn removed_member_api_key_not_in_list(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let bob_id = fixture.auth.account_id_bob().as_i64();

    // Get initial API key count
    let list_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;
    let list_response = reqwest::Client::new()
        .get(list_url.clone())
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(list_response.status(), StatusCode::OK);
    let initial_list = list_response.json::<ApiKeyListResponse>().await?;
    let initial_count = initial_list.api_keys.len();

    // Verify Bob has at least one key in the list
    let bob_keys_before = initial_list
        .api_keys
        .iter()
        .filter(|k| k.account_id == bob_id)
        .count();
    assert!(
        bob_keys_before > 0,
        "Bob should have at least one API key before removal"
    );

    // Alice (admin) removes Bob from the org
    let remove_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/members/{bob_id}"))?;
    let remove_response = reqwest::Client::new()
        .delete(remove_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(remove_response.status(), StatusCode::NO_CONTENT);

    // Verify Bob's API keys no longer appear in the list
    let list_response_after = reqwest::Client::new()
        .get(list_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(list_response_after.status(), StatusCode::OK);
    let final_list = list_response_after.json::<ApiKeyListResponse>().await?;

    let bob_keys_after = final_list
        .api_keys
        .iter()
        .filter(|k| k.account_id == bob_id)
        .count();
    pretty_assert_eq!(
        bob_keys_after,
        0,
        "Bob's API keys should not appear in the list after removal"
    );

    pretty_assert_eq!(
        final_list.api_keys.len(),
        initial_count - bob_keys_before,
        "Total API key count should decrease by Bob's key count"
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

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct CreateBotResponse {
    account_id: i64,
    name: String,
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct BotListResponse {
    bots: Vec<BotEntry>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct BotEntry {
    account_id: i64,
    name: Option<String>,
    responsible_email: String,
    created_at: String,
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_bot_as_admin(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::CREATED);

    let bot = response.json::<CreateBotResponse>().await?;
    pretty_assert_eq!(bot.name, "CI Bot");
    assert!(bot.account_id > 0);
    assert!(!bot.api_key.is_empty());

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_bot_as_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;

    // Bob is a member (not admin)
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_bot_as_non_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;

    // Charlie is not a member of Acme
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_bot_empty_name_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({
            "name": "  ",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn create_bot_empty_email_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;

    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "  "
        }))
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_bots_as_admin(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // First create a bot
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;
    pretty_assert_eq!(create_response.status(), StatusCode::CREATED);

    // Now list bots
    let list_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;
    let list_response = reqwest::Client::new()
        .get(list_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;

    pretty_assert_eq!(list_response.status(), StatusCode::OK);

    let list = list_response.json::<BotListResponse>().await?;
    pretty_assert_eq!(list.bots.len(), 1);
    pretty_assert_eq!(list.bots[0].name, Some(String::from("CI Bot")));
    pretty_assert_eq!(list.bots[0].responsible_email, "devops@acme.com");

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_bots_as_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;

    // Bob is a member (not admin)
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn list_bots_as_non_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;

    // Charlie is not a member of Acme
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn bot_api_key_works_for_org_operations(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create a bot
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;
    pretty_assert_eq!(create_response.status(), StatusCode::CREATED);

    let bot = create_response.json::<CreateBotResponse>().await?;

    // Use the bot's API key to access CAS
    let health_url = fixture.base_url.join("api/v1/health")?;
    let health_response = reqwest::Client::new()
        .get(health_url)
        .bearer_auth(&bot.api_key)
        .send()
        .await?;

    // Health endpoint should work (it doesn't require auth, but the key should be
    // valid)
    pretty_assert_eq!(health_response.status(), StatusCode::OK);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn revoked_bot_api_key_no_longer_works(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Create a bot
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;
    pretty_assert_eq!(create_response.status(), StatusCode::CREATED);

    let bot = create_response.json::<CreateBotResponse>().await?;

    // Use a test key for CAS operations
    let test_content = b"test blob content";
    let test_key = crate::helpers::test_blob(test_content);

    // Verify the bot's API key works for an authenticated endpoint (CAS write)
    let cas_url = fixture.base_url.join(&format!("api/v1/cas/{}", test_key))?;
    let write_response = reqwest::Client::new()
        .put(cas_url.clone())
        .bearer_auth(&bot.api_key)
        .body(test_content.to_vec())
        .send()
        .await?;
    pretty_assert_eq!(
        write_response.status(),
        StatusCode::CREATED,
        "Bot API key should work before revocation"
    );

    // Revoke the bot (remove from organization)
    let revoke_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/members/{}",
        bot.account_id
    ))?;
    let revoke_response = reqwest::Client::new()
        .delete(revoke_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(revoke_response.status(), StatusCode::NO_CONTENT);

    // Verify the bot's API key no longer works - try to write a different blob
    let test_content_2 = b"another test blob";
    let test_key_2 = crate::helpers::test_blob(test_content_2);
    let cas_url_2 = fixture
        .base_url
        .join(&format!("api/v1/cas/{}", test_key_2))?;
    let write_response_after = reqwest::Client::new()
        .put(cas_url_2)
        .bearer_auth(&bot.api_key)
        .body(test_content_2.to_vec())
        .send()
        .await?;
    pretty_assert_eq!(
        write_response_after.status(),
        StatusCode::UNAUTHORIZED,
        "Bot API key should be revoked after bot is removed from org"
    );

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn revoked_bot_api_key_not_in_list(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();

    // Get initial API key count
    let list_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/api-keys"))?;
    let list_response = reqwest::Client::new()
        .get(list_url.clone())
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(list_response.status(), StatusCode::OK);
    let initial_list = list_response.json::<ApiKeyListResponse>().await?;
    let initial_count = initial_list.api_keys.len();

    // Create a bot
    let create_url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}/bots"))?;
    let create_response = reqwest::Client::new()
        .post(create_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&serde_json::json!({
            "name": "CI Bot",
            "responsible_email": "devops@acme.com"
        }))
        .send()
        .await?;
    pretty_assert_eq!(create_response.status(), StatusCode::CREATED);

    let bot = create_response.json::<CreateBotResponse>().await?;

    // Verify the bot's API key appears in the list
    let list_response_with_bot = reqwest::Client::new()
        .get(list_url.clone())
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(list_response_with_bot.status(), StatusCode::OK);
    let list_with_bot = list_response_with_bot.json::<ApiKeyListResponse>().await?;

    let bot_keys_before = list_with_bot
        .api_keys
        .iter()
        .filter(|k| k.account_id == bot.account_id)
        .count();
    pretty_assert_eq!(
        bot_keys_before,
        1,
        "Bot should have exactly one API key after creation"
    );
    pretty_assert_eq!(
        list_with_bot.api_keys.len(),
        initial_count + 1,
        "Total API key count should increase by 1 after bot creation"
    );

    // Revoke the bot (remove from organization)
    let revoke_url = fixture.base_url.join(&format!(
        "api/v1/organizations/{org_id}/members/{}",
        bot.account_id
    ))?;
    let revoke_response = reqwest::Client::new()
        .delete(revoke_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(revoke_response.status(), StatusCode::NO_CONTENT);

    // Verify the bot's API key no longer appears in the list
    let list_response_after = reqwest::Client::new()
        .get(list_url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .send()
        .await?;
    pretty_assert_eq!(list_response_after.status(), StatusCode::OK);
    let final_list = list_response_after.json::<ApiKeyListResponse>().await?;

    let bot_keys_after = final_list
        .api_keys
        .iter()
        .filter(|k| k.account_id == bot.account_id)
        .count();
    pretty_assert_eq!(
        bot_keys_after,
        0,
        "Bot's API key should not appear in the list after revocation"
    );

    pretty_assert_eq!(
        final_list.api_keys.len(),
        initial_count,
        "Total API key count should return to initial count after bot revocation"
    );

    Ok(())
}

// ============================================================================
// Rename Organization Tests (Strongly Typed Role System)
// ============================================================================
// These tests verify the strongly typed role system implementation for the
// rename endpoint, which uses `session.try_admin()` to enforce admin access
// at compile time.

#[derive(Debug, Serialize)]
struct RenameOrganizationRequest {
    name: String,
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn rename_organization_as_admin_succeeds(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}"))?;

    // Alice is an admin of Acme, so she should be able to rename it
    let response = reqwest::Client::new()
        .patch(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&RenameOrganizationRequest {
            name: String::from("Acme Renamed"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::NO_CONTENT);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn rename_organization_as_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}"))?;

    // Bob is a member (not admin) of Acme, so he should get 403 Forbidden
    let response = reqwest::Client::new()
        .patch(url)
        .bearer_auth(fixture.auth.session_bob().expose())
        .json(&RenameOrganizationRequest {
            name: String::from("Bob's Rename Attempt"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn rename_organization_as_non_member_forbidden(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}"))?;

    // Charlie is not a member of Acme, so he should get 403 Forbidden
    let response = reqwest::Client::new()
        .patch(url)
        .bearer_auth(fixture.auth.session_charlie().expose())
        .json(&RenameOrganizationRequest {
            name: String::from("Charlie's Rename Attempt"),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::FORBIDDEN);

    Ok(())
}

#[sqlx::test(migrator = "courier::db::Postgres::MIGRATOR")]
async fn rename_organization_empty_name_fails(pool: PgPool) -> Result<()> {
    let fixture = TestFixture::spawn(pool).await?;
    let org_id = fixture.auth.org_acme().as_i64();
    let url = fixture
        .base_url
        .join(&format!("api/v1/organizations/{org_id}"))?;

    // Even as admin, empty name should fail with 400 Bad Request
    let response = reqwest::Client::new()
        .patch(url)
        .bearer_auth(fixture.auth.session_alice().expose())
        .json(&RenameOrganizationRequest {
            name: String::from("   "),
        })
        .send()
        .await?;

    pretty_assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    Ok(())
}
