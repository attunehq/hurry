//! Tests for invitation database operations.

use courier::{
    auth::OrgRole,
    crypto,
    db::{AcceptInvitationResult, Postgres},
};
use pretty_assertions::assert_eq as pretty_assert_eq;
use time::{Duration, OffsetDateTime};

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn create_and_get_invitation(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    let invitation_id = db
        .create_invitation(
            org_id,
            &token,
            OrgRole::Member,
            creator_id,
            Some(expires_at),
            Some(10),
        )
        .await
        .unwrap();

    // Get by token
    let invitation = db.get_invitation_by_token(&token).await.unwrap().unwrap();

    pretty_assert_eq!(invitation.id, invitation_id);
    pretty_assert_eq!(invitation.organization_id, org_id);
    pretty_assert_eq!(invitation.role, OrgRole::Member);
    pretty_assert_eq!(invitation.created_by, creator_id);
    pretty_assert_eq!(invitation.max_uses, Some(10));
    pretty_assert_eq!(invitation.use_count, 0);
    assert!(invitation.revoked_at.is_none());
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_invitation_preview(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Organization").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    db.create_invitation(
        org_id,
        &token,
        OrgRole::Admin,
        creator_id,
        Some(expires_at),
        None,
    )
    .await
    .unwrap();

    let preview = db.get_invitation_preview(&token).await.unwrap().unwrap();

    pretty_assert_eq!(preview.organization_name, "Test Organization");
    pretty_assert_eq!(preview.role, OrgRole::Admin);
    assert!(preview.valid);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn get_expired_invitation_preview(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() - Duration::hours(1); // Expired

    db.create_invitation(
        org_id,
        &token,
        OrgRole::Member,
        creator_id,
        Some(expires_at),
        None,
    )
    .await
    .unwrap();

    let preview = db.get_invitation_preview(&token).await.unwrap().unwrap();

    assert!(!preview.valid);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn accept_invitation_success(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();
    let joiner_id = db.create_account("joiner@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    db.create_invitation(
        org_id,
        &token,
        OrgRole::Member,
        creator_id,
        Some(expires_at),
        Some(10),
    )
    .await
    .unwrap();

    // Accept invitation
    let result = db.accept_invitation(&token, joiner_id).await.unwrap();

    match result {
        AcceptInvitationResult::Success {
            organization_id,
            organization_name,
            role,
        } => {
            pretty_assert_eq!(organization_id, org_id);
            pretty_assert_eq!(organization_name, "Test Org");
            pretty_assert_eq!(role, OrgRole::Member);
        }
        other => panic!("Expected Success, got {:?}", other),
    }

    // Verify membership was created
    let role = db.get_member_role(org_id, joiner_id).await.unwrap();
    pretty_assert_eq!(role, Some(OrgRole::Member));

    // Verify use count was incremented
    let invitation = db.get_invitation_by_token(&token).await.unwrap().unwrap();
    pretty_assert_eq!(invitation.use_count, 1);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn accept_invitation_not_found(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let _org_id = db.create_organization("Test Org").await.unwrap();
    let account_id = db.create_account("test@test.com", None).await.unwrap();

    let result = db
        .accept_invitation("nonexistent_token", account_id)
        .await
        .unwrap();

    assert!(matches!(result, AcceptInvitationResult::NotFound));
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn accept_invitation_expired(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();
    let joiner_id = db.create_account("joiner@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() - Duration::hours(1); // Expired

    db.create_invitation(
        org_id,
        &token,
        OrgRole::Member,
        creator_id,
        Some(expires_at),
        None,
    )
    .await
    .unwrap();

    let result = db.accept_invitation(&token, joiner_id).await.unwrap();

    assert!(matches!(result, AcceptInvitationResult::Expired));
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn accept_invitation_revoked(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();
    let joiner_id = db.create_account("joiner@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    let invitation_id = db
        .create_invitation(
            org_id,
            &token,
            OrgRole::Member,
            creator_id,
            Some(expires_at),
            None,
        )
        .await
        .unwrap();

    // Revoke it
    db.revoke_invitation(invitation_id).await.unwrap();

    let result = db.accept_invitation(&token, joiner_id).await.unwrap();

    assert!(matches!(result, AcceptInvitationResult::Revoked));
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn accept_invitation_max_uses_reached(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();
    let joiner1_id = db.create_account("joiner1@test.com", None).await.unwrap();
    let joiner2_id = db.create_account("joiner2@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    db.create_invitation(
        org_id,
        &token,
        OrgRole::Member,
        creator_id,
        Some(expires_at),
        Some(1),
    )
    .await
    .unwrap();

    // First use succeeds
    let result = db.accept_invitation(&token, joiner1_id).await.unwrap();
    assert!(matches!(result, AcceptInvitationResult::Success { .. }));

    // Second use fails
    let result = db.accept_invitation(&token, joiner2_id).await.unwrap();
    assert!(matches!(result, AcceptInvitationResult::MaxUsesReached));
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn accept_invitation_already_member(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();
    let joiner_id = db.create_account("joiner@test.com", None).await.unwrap();

    // Add as member first
    db.add_organization_member(org_id, joiner_id, OrgRole::Member)
        .await
        .unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    db.create_invitation(
        org_id,
        &token,
        OrgRole::Admin,
        creator_id,
        Some(expires_at),
        None,
    )
    .await
    .unwrap();

    let result = db.accept_invitation(&token, joiner_id).await.unwrap();

    assert!(matches!(result, AcceptInvitationResult::AlreadyMember));
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn revoke_invitation(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();

    let token = crypto::generate_invitation_token(false);
    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    let invitation_id = db
        .create_invitation(
            org_id,
            &token,
            OrgRole::Member,
            creator_id,
            Some(expires_at),
            None,
        )
        .await
        .unwrap();

    let revoked = db.revoke_invitation(invitation_id).await.unwrap();
    assert!(revoked);

    // Verify revoked
    let invitation = db.get_invitation_by_token(&token).await.unwrap().unwrap();
    assert!(invitation.revoked_at.is_some());

    // Revoking again returns false
    let revoked_again = db.revoke_invitation(invitation_id).await.unwrap();
    assert!(!revoked_again);
}

#[sqlx::test(migrator = "Postgres::MIGRATOR")]
async fn list_invitations(pool: sqlx::PgPool) {
    let db = Postgres { pool };

    let org_id = db.create_organization("Test Org").await.unwrap();
    let creator_id = db.create_account("creator@test.com", None).await.unwrap();

    let expires_at = OffsetDateTime::now_utc() + Duration::days(7);

    db.create_invitation(
        org_id,
        &crypto::generate_invitation_token(false),
        OrgRole::Member,
        creator_id,
        Some(expires_at),
        None,
    )
    .await
    .unwrap();

    db.create_invitation(
        org_id,
        &crypto::generate_invitation_token(false),
        OrgRole::Admin,
        creator_id,
        Some(expires_at),
        Some(5),
    )
    .await
    .unwrap();

    let invitations = db.list_invitations(org_id).await.unwrap();

    pretty_assert_eq!(invitations.len(), 2);
}
