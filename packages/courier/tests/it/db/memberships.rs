//! Tests for organization membership database operations.

use courier::{auth::OrgRole, db::Postgres};
use pretty_assertions::assert_eq as pretty_assert_eq;

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn add_and_get_member_role(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Add as member
    db.add_organization_member(org_id, account_id, OrgRole::Member)
        .await
        .unwrap();

    // Get role
    let role = db.get_member_role(org_id, account_id).await.unwrap();

    pretty_assert_eq!(role, Some(OrgRole::Member));
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_role_for_nonmember(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Don't add membership

    let role = db.get_member_role(org_id, account_id).await.unwrap();

    assert!(role.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn update_member_role(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    db.add_organization_member(org_id, account_id, OrgRole::Member)
        .await
        .unwrap();

    // Promote to admin
    let updated = db
        .update_member_role(org_id, account_id, OrgRole::Admin)
        .await
        .unwrap();

    assert!(updated);

    let role = db.get_member_role(org_id, account_id).await.unwrap();
    pretty_assert_eq!(role, Some(OrgRole::Admin));
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn update_nonexistent_member_role(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // No membership exists

    let updated = db
        .update_member_role(org_id, account_id, OrgRole::Admin)
        .await
        .unwrap();

    assert!(!updated);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn remove_organization_member(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    db.add_organization_member(org_id, account_id, OrgRole::Member)
        .await
        .unwrap();

    // Remove membership
    let removed = db
        .remove_organization_member(org_id, account_id)
        .await
        .unwrap();

    assert!(removed);

    // Should no longer be a member
    let role = db.get_member_role(org_id, account_id).await.unwrap();
    assert!(role.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn remove_nonexistent_member(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // No membership exists

    let removed = db
        .remove_organization_member(org_id, account_id)
        .await
        .unwrap();

    assert!(!removed);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn list_organization_members(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();

    let alice_id = db
        .create_account("alice@test.com", Some("Alice"))
        .await
        .unwrap();
    let bob_id = db
        .create_account("bob@test.com", Some("Bob"))
        .await
        .unwrap();

    db.add_organization_member(org_id, alice_id, OrgRole::Admin)
        .await
        .unwrap();
    db.add_organization_member(org_id, bob_id, OrgRole::Member)
        .await
        .unwrap();

    let members = db.list_organization_members(org_id).await.unwrap();

    pretty_assert_eq!(members.len(), 2);

    // Sorted by email
    pretty_assert_eq!(members[0].email, "alice@test.com");
    pretty_assert_eq!(members[0].role, OrgRole::Admin);
    pretty_assert_eq!(members[1].email, "bob@test.com");
    pretty_assert_eq!(members[1].role, OrgRole::Member);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn is_last_admin_true(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();

    let admin_id = db.create_account("admin@test.com", None).await.unwrap();
    let member_id = db.create_account("member@test.com", None).await.unwrap();

    // Link GitHub identity to make admin a "human" (not a bot)
    db.link_github_identity(admin_id, 12345, "admin_user")
        .await
        .unwrap();

    db.add_organization_member(org_id, admin_id, OrgRole::Admin)
        .await
        .unwrap();
    db.add_organization_member(org_id, member_id, OrgRole::Member)
        .await
        .unwrap();

    // admin is the last human admin
    let is_last = db.is_last_admin(org_id, admin_id).await.unwrap();
    assert!(is_last);

    // member is not an admin
    let is_last = db.is_last_admin(org_id, member_id).await.unwrap();
    assert!(!is_last);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn is_last_admin_false_with_multiple_admins(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();

    let admin1_id = db.create_account("admin1@test.com", None).await.unwrap();
    let admin2_id = db.create_account("admin2@test.com", None).await.unwrap();

    // Link GitHub identities to make both admins "humans" (not bots)
    db.link_github_identity(admin1_id, 12345, "admin1_user")
        .await
        .unwrap();
    db.link_github_identity(admin2_id, 67890, "admin2_user")
        .await
        .unwrap();

    db.add_organization_member(org_id, admin1_id, OrgRole::Admin)
        .await
        .unwrap();
    db.add_organization_member(org_id, admin2_id, OrgRole::Admin)
        .await
        .unwrap();

    // Neither is the last human admin
    let is_last = db.is_last_admin(org_id, admin1_id).await.unwrap();
    assert!(!is_last);

    let is_last = db.is_last_admin(org_id, admin2_id).await.unwrap();
    assert!(!is_last);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn is_last_admin_ignores_bot_admins(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();

    let human_admin_id = db.create_account("human@test.com", None).await.unwrap();
    let bot_admin_id = db.create_account("bot@test.com", None).await.unwrap();

    // Only link GitHub identity to the human admin
    db.link_github_identity(human_admin_id, 12345, "human_user")
        .await
        .unwrap();

    db.add_organization_member(org_id, human_admin_id, OrgRole::Admin)
        .await
        .unwrap();
    db.add_organization_member(org_id, bot_admin_id, OrgRole::Admin)
        .await
        .unwrap();

    // Human admin is the last human admin (bot admin doesn't count)
    let is_last = db.is_last_admin(org_id, human_admin_id).await.unwrap();
    assert!(is_last);

    // Bot admin is not considered a human admin
    let is_last = db.is_last_admin(org_id, bot_admin_id).await.unwrap();
    assert!(!is_last);
}
