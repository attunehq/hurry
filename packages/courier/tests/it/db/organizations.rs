//! Tests for organization database operations.

use courier::{auth::OrgRole, db::Postgres};
use pretty_assertions::assert_eq as pretty_assert_eq;

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn create_and_get_organization(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Organization").await.unwrap();

    let org = db.get_organization(org_id).await.unwrap().unwrap();

    pretty_assert_eq!(org.id, org_id);
    pretty_assert_eq!(org.name, "Test Organization");
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_nonexistent_organization(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org = db
        .get_organization(courier::auth::OrgId::from_i64(99999))
        .await
        .unwrap();

    assert!(org.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn list_organizations_for_account(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    // Create orgs
    let org1_id = db.create_organization("Org 1").await.unwrap();
    let org2_id = db.create_organization("Org 2").await.unwrap();

    // Create account
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Add memberships
    db.add_organization_member(org1_id, account_id, OrgRole::Admin)
        .await
        .unwrap();
    db.add_organization_member(org2_id, account_id, OrgRole::Member)
        .await
        .unwrap();

    // List organizations
    let orgs = db.list_organizations_for_account(account_id).await.unwrap();

    pretty_assert_eq!(orgs.len(), 2);

    // Sorted by name
    pretty_assert_eq!(orgs[0].organization.name, "Org 1");
    pretty_assert_eq!(orgs[0].role, OrgRole::Admin);
    pretty_assert_eq!(orgs[1].organization.name, "Org 2");
    pretty_assert_eq!(orgs[1].role, OrgRole::Member);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn list_organizations_for_account_with_no_memberships(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let account_id = db.create_account("test@test.com", None).await.unwrap();

    // Don't add any memberships

    let orgs = db.list_organizations_for_account(account_id).await.unwrap();

    assert!(orgs.is_empty());
}
