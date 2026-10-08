// Copyright (C) 2026 Marcos Gabriel Miller
//! End-to-end check against a running backend. Ignored by default:
//! `MM_LIVE_URL=http://127.0.0.1:8000/api/v1 MM_LIVE_USER=default MM_LIVE_PASSWORD=… cargo test live -- --ignored`
use std::time::Duration;

use chrono::Utc;
use rust_decimal::Decimal;

use crate::{
    api::{ApiClient, models::*},
    session::{SessionManager, catalog::EXPENSE_CODE, store::memory_store},
};

#[tokio::test]
#[ignore = "needs a running backend (MM_LIVE_URL)"]
async fn live_backend_round_trip() {
    let url = std::env::var("MM_LIVE_URL").expect("MM_LIVE_URL");
    let user = std::env::var("MM_LIVE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("MM_LIVE_PASSWORD").expect("MM_LIVE_PASSWORD");
    let api = ApiClient::new(&url, Duration::from_secs(10)).unwrap();
    let sessions = SessionManager::new(api.clone(), memory_store().await, Some([9; 32]), None);
    let uid = 1;

    let login = sessions.login(uid, uid, &user, &password).await.unwrap();
    let auth = sessions.auth(uid).await.unwrap();
    assert_eq!(auth.user_uuid, login.user.uuid);

    let accounts = api
        .list_accounts(&auth.token, auth.user_uuid)
        .await
        .unwrap();
    let categories = api
        .list_categories(&auth.token, auth.user_uuid)
        .await
        .unwrap();
    let types = api.list_types(&auth.token).await.unwrap();
    assert!(!api.list_currencies(&auth.token).await.unwrap().is_empty());
    let account = accounts.first().expect("user has an account");
    let category = categories.first().expect("user has a category");
    let out = types
        .iter()
        .find(|t| t.code == EXPENSE_CODE)
        .expect("OUT type");

    let created = api
        .create_transaction(
            &auth.token,
            &CreateTransaction {
                amount: Decimal::new(123456, 2),
                description: Some("bot live test".into()),
                transaction_date: Utc::now(),
                type_uuid: out.uuid,
                category_uuid: Some(category.uuid),
                account_uuid: account.uuid,
            },
        )
        .await
        .unwrap();
    assert_eq!(created.amount, Decimal::new(123456, 2));

    let listed = api
        .list_transactions(
            &auth.token,
            auth.user_uuid,
            &TransactionQuery {
                search: Some("bot live test".into()),
                ..Default::default()
            },
            1,
            10,
        )
        .await
        .unwrap();
    assert!(listed.data.iter().any(|tx| tx.uuid == created.uuid));

    let updated = api
        .update_transaction(
            &auth.token,
            created.uuid,
            &UpdateTransaction {
                amount: Some(Decimal::from(10)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.amount, Decimal::from(10));
    assert_eq!(
        updated.category_uuid,
        Some(category.uuid),
        "PATCH must keep the category (BK-2)"
    );

    let balance = api
        .report_balance(&auth.token, auth.user_uuid, Some(account.uuid))
        .await
        .unwrap();
    assert_eq!(balance.data.len(), 1);
    let by_category = api
        .report_by_category(
            &auth.token,
            auth.user_uuid,
            &CategoryReportQuery {
                account_uuid: Some(account.uuid),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(
        by_category
            .data
            .iter()
            .any(|row| row.category_uuid == Some(category.uuid))
    );

    api.delete_transaction(&auth.token, created.uuid)
        .await
        .unwrap();
    assert_eq!(
        api.get_transaction(&auth.token, created.uuid)
            .await
            .unwrap_err()
            .status(),
        Some(404)
    );

    sessions.logout(uid).await.unwrap();
    assert_eq!(
        api.list_currencies(&auth.token).await.unwrap_err().status(),
        Some(401),
        "token revoked (BK-5)"
    );
}

/// `MM_LIVE_JWT_SECRET` must be the backend's `JWT_SECRET`.
#[tokio::test]
#[ignore = "needs a running backend (MM_LIVE_URL, MM_LIVE_JWT_SECRET)"]
async fn live_telegram_auth() {
    use crate::{config::TelegramAuth, session::minter::TokenMinter};
    use std::collections::HashMap;

    let url = std::env::var("MM_LIVE_URL").expect("MM_LIVE_URL");
    let secret = std::env::var("MM_LIVE_JWT_SECRET").expect("MM_LIVE_JWT_SECRET");
    let user = std::env::var("MM_LIVE_USER").unwrap_or_else(|_| "default".into());
    let password = std::env::var("MM_LIVE_PASSWORD").expect("MM_LIVE_PASSWORD");
    let api = ApiClient::new(&url, Duration::from_secs(10)).unwrap();
    // Only to discover the user's UUID for TELEGRAM_USERS.
    let user_uuid = api.login(&user, &password).await.unwrap().user.uuid;

    let auth = TelegramAuth {
        jwt_secret: secret,
        users: HashMap::from([(777, user_uuid)]),
        token_ttl: Duration::from_secs(300),
    };
    let sessions = SessionManager::new(
        api.clone(),
        memory_store().await,
        None,
        Some(TokenMinter::new(&auth)),
    );

    let logged = sessions.login_telegram(777, 777).await.unwrap();
    assert_eq!(logged.uuid, user_uuid);
    let session = sessions.auth(777).await.unwrap();

    let accounts = api.list_accounts(&session.token, user_uuid).await.unwrap();
    let out = api
        .list_types(&session.token)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.code == EXPENSE_CODE)
        .unwrap();
    let created = api
        .create_transaction(
            &session.token,
            &CreateTransaction {
                amount: Decimal::from(42),
                description: Some("bot telegram auth test".into()),
                transaction_date: Utc::now(),
                type_uuid: out.uuid,
                category_uuid: None,
                account_uuid: accounts[0].uuid,
            },
        )
        .await
        .unwrap();
    api.delete_transaction(&session.token, created.uuid)
        .await
        .unwrap();

    assert!(
        sessions.login_telegram(778, 778).await.is_err(),
        "unmapped Telegram id"
    );

    sessions.logout(777).await.unwrap();
    assert_eq!(
        api.list_currencies(&session.token)
            .await
            .unwrap_err()
            .status(),
        Some(401)
    );
}
