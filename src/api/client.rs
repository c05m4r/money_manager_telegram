// Copyright (C) 2026 Marcos Gabriel Miller
use std::time::Duration;

use reqwest::{Method, RequestBuilder};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use super::{error::ApiError, models::*};

/// Max page size the backend accepts.
const MAX_PER_PAGE: u64 = 100;

/// Thin typed client over `money_manager_backend` `/api/v1`. The only place that knows URLs.
///
/// Every list call is scoped with `user_uuid` so `admin`/`auditor` sessions see their own
/// finances in the bot, not every user's.
#[derive(Clone)]
pub struct ApiClient {
    http: reqwest::Client,
    base_url: String,
}

impl ApiClient {
    pub fn new(base_url: &str, timeout: Duration) -> Result<Self, ApiError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(concat!(
                "money_manager_telegram/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()?;
        Ok(Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }

    fn request(&self, method: Method, path: &str, token: Option<&str>) -> RequestBuilder {
        let builder = self
            .http
            .request(method, format!("{}{}", self.base_url, path));
        match token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    async fn send(&self, builder: RequestBuilder) -> Result<String, ApiError> {
        let response = builder.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(ApiError::from_body(status.as_u16(), &body))
        }
    }

    async fn json<T: DeserializeOwned>(&self, builder: RequestBuilder) -> Result<T, ApiError> {
        let body = self.send(builder).await?;
        serde_json::from_str(&body).map_err(|error| ApiError::Decode(error.to_string()))
    }

    async fn get<T: DeserializeOwned>(
        &self,
        token: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ApiError> {
        self.json(self.request(Method::GET, path, Some(token)).query(query))
            .await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        token: &str,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.json(self.request(Method::POST, path, Some(token)).json(body))
            .await
    }

    async fn patch<B: Serialize, T: DeserializeOwned>(
        &self,
        token: &str,
        path: &str,
        body: &B,
    ) -> Result<T, ApiError> {
        self.json(self.request(Method::PATCH, path, Some(token)).json(body))
            .await
    }

    async fn delete(&self, token: &str, path: &str) -> Result<(), ApiError> {
        self.send(self.request(Method::DELETE, path, Some(token)))
            .await
            .map(|_| ())
    }

    /// Walks every page of a paginated list endpoint.
    async fn fetch_all<T: DeserializeOwned>(
        &self,
        token: &str,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Vec<T>, ApiError> {
        let mut items = Vec::new();
        let mut page = 1;
        loop {
            let mut params = query.to_vec();
            params.push(("page", page.to_string()));
            params.push(("per_page", MAX_PER_PAGE.to_string()));
            let response: Paginated<T> = self.get(token, path, &params).await?;
            items.extend(response.data);
            if page >= response.meta.total_pages {
                return Ok(items);
            }
            page += 1;
        }
    }

    // ---------- auth / users ----------

    /// `identifier` is sent as `email` when it contains `@`, otherwise as `username`.
    pub async fn login(&self, identifier: &str, password: &str) -> Result<LoginResponse, ApiError> {
        let is_email = identifier.contains('@');
        let body = LoginRequest {
            username: (!is_email).then_some(identifier),
            email: is_email.then_some(identifier),
            password,
        };
        self.json(self.request(Method::POST, "/auth/login", None).json(&body))
            .await
    }

    pub async fn logout(&self, token: &str) -> Result<(), ApiError> {
        self.send(self.request(Method::POST, "/auth/logout", Some(token)))
            .await
            .map(|_| ())
    }

    pub async fn get_user(&self, token: &str, uuid: Uuid) -> Result<User, ApiError> {
        self.get(token, &format!("/users/{uuid}"), &[]).await
    }

    // ---------- currencies ----------

    pub async fn list_currencies(&self, token: &str) -> Result<Vec<Currency>, ApiError> {
        self.fetch_all(token, "/currencies", &[]).await
    }

    pub async fn create_currency(
        &self,
        token: &str,
        body: &CreateCurrency,
    ) -> Result<Currency, ApiError> {
        self.post(token, "/currencies", body).await
    }

    pub async fn update_currency(
        &self,
        token: &str,
        code: &str,
        body: &UpdateCurrency,
    ) -> Result<Currency, ApiError> {
        self.patch(token, &format!("/currencies/{code}"), body)
            .await
    }

    pub async fn delete_currency(&self, token: &str, code: &str) -> Result<(), ApiError> {
        self.delete(token, &format!("/currencies/{code}")).await
    }

    // ---------- transaction types ----------

    pub async fn list_types(&self, token: &str) -> Result<Vec<TransactionType>, ApiError> {
        self.fetch_all(token, "/transaction-types", &[]).await
    }

    pub async fn create_type(
        &self,
        token: &str,
        body: &CreateTransactionType,
    ) -> Result<TransactionType, ApiError> {
        self.post(token, "/transaction-types", body).await
    }

    pub async fn update_type(
        &self,
        token: &str,
        uuid: Uuid,
        body: &UpdateTransactionType,
    ) -> Result<TransactionType, ApiError> {
        self.patch(token, &format!("/transaction-types/{uuid}"), body)
            .await
    }

    pub async fn delete_type(&self, token: &str, uuid: Uuid) -> Result<(), ApiError> {
        self.delete(token, &format!("/transaction-types/{uuid}"))
            .await
    }

    // ---------- accounts ----------

    pub async fn list_accounts(
        &self,
        token: &str,
        user_uuid: Uuid,
    ) -> Result<Vec<Account>, ApiError> {
        self.fetch_all(token, "/accounts", &[("user_uuid", user_uuid.to_string())])
            .await
    }

    pub async fn create_account(
        &self,
        token: &str,
        body: &CreateAccount,
    ) -> Result<Account, ApiError> {
        self.post(token, "/accounts", body).await
    }

    pub async fn update_account(
        &self,
        token: &str,
        uuid: Uuid,
        body: &UpdateAccount,
    ) -> Result<Account, ApiError> {
        self.patch(token, &format!("/accounts/{uuid}"), body).await
    }

    pub async fn delete_account(&self, token: &str, uuid: Uuid) -> Result<(), ApiError> {
        self.delete(token, &format!("/accounts/{uuid}")).await
    }

    // ---------- categories ----------

    pub async fn list_categories(
        &self,
        token: &str,
        user_uuid: Uuid,
    ) -> Result<Vec<Category>, ApiError> {
        self.fetch_all(
            token,
            "/categories",
            &[("user_uuid", user_uuid.to_string())],
        )
        .await
    }

    pub async fn create_category(
        &self,
        token: &str,
        body: &CreateCategory,
    ) -> Result<Category, ApiError> {
        self.post(token, "/categories", body).await
    }

    pub async fn update_category(
        &self,
        token: &str,
        uuid: Uuid,
        body: &UpdateCategory,
    ) -> Result<Category, ApiError> {
        self.patch(token, &format!("/categories/{uuid}"), body)
            .await
    }

    pub async fn delete_category(&self, token: &str, uuid: Uuid) -> Result<(), ApiError> {
        self.delete(token, &format!("/categories/{uuid}")).await
    }

    // ---------- transactions ----------

    pub async fn list_transactions(
        &self,
        token: &str,
        user_uuid: Uuid,
        query: &TransactionQuery,
        page: u64,
        per_page: u64,
    ) -> Result<Paginated<Transaction>, ApiError> {
        let mut params = vec![
            ("user_uuid", user_uuid.to_string()),
            ("sort", "-transaction_date".to_string()),
            ("page", page.max(1).to_string()),
            ("per_page", per_page.clamp(1, MAX_PER_PAGE).to_string()),
        ];
        if let Some(value) = query.account_uuid {
            params.push(("account_uuid", value.to_string()));
        }
        if let Some(value) = query.category_uuid {
            params.push(("category_uuid", value.to_string()));
        }
        if let Some(value) = query.type_uuid {
            params.push(("type_uuid", value.to_string()));
        }
        if let Some(value) = query.date_from {
            params.push(("date_from", value.to_string()));
        }
        if let Some(value) = query.date_to {
            params.push(("date_to", value.to_string()));
        }
        if let Some(value) = &query.search {
            params.push(("search", value.clone()));
        }
        self.get(token, "/transactions", &params).await
    }

    /// Number of active transactions linked to an account or category.
    pub async fn count_transactions(
        &self,
        token: &str,
        user_uuid: Uuid,
        query: &TransactionQuery,
    ) -> Result<u64, ApiError> {
        Ok(self
            .list_transactions(token, user_uuid, query, 1, 1)
            .await?
            .meta
            .total_records)
    }

    pub async fn get_transaction(&self, token: &str, uuid: Uuid) -> Result<Transaction, ApiError> {
        self.get(token, &format!("/transactions/{uuid}"), &[]).await
    }

    pub async fn create_transaction(
        &self,
        token: &str,
        body: &CreateTransaction,
    ) -> Result<Transaction, ApiError> {
        self.post(token, "/transactions", body).await
    }

    pub async fn update_transaction(
        &self,
        token: &str,
        uuid: Uuid,
        body: &UpdateTransaction,
    ) -> Result<Transaction, ApiError> {
        self.patch(token, &format!("/transactions/{uuid}"), body)
            .await
    }

    pub async fn delete_transaction(&self, token: &str, uuid: Uuid) -> Result<(), ApiError> {
        self.delete(token, &format!("/transactions/{uuid}")).await
    }

    // ---------- reports ----------

    pub async fn report_balance(
        &self,
        token: &str,
        user_uuid: Uuid,
        account_uuid: Option<Uuid>,
    ) -> Result<BalanceReport, ApiError> {
        let mut params = vec![("user_uuid", user_uuid.to_string())];
        if let Some(account_uuid) = account_uuid {
            params.push(("account_uuid", account_uuid.to_string()));
        }
        self.get(token, "/reports/balance", &params).await
    }

    pub async fn report_by_category(
        &self,
        token: &str,
        user_uuid: Uuid,
        query: &CategoryReportQuery,
    ) -> Result<CategoryReport, ApiError> {
        let mut params = vec![("user_uuid", user_uuid.to_string())];
        if let Some(value) = query.account_uuid {
            params.push(("account_uuid", value.to_string()));
        }
        if let Some(value) = query.from {
            params.push((
                "from",
                value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ));
        }
        if let Some(value) = query.to {
            params.push((
                "to",
                value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ));
        }
        if let Some(value) = &query.type_code {
            params.push(("type_code", value.clone()));
        }
        self.get(token, "/reports/by-category", &params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    fn client(server: &MockServer) -> ApiClient {
        ApiClient::new(&server.uri(), Duration::from_secs(5)).unwrap()
    }

    fn page(data: serde_json::Value, current: u64, total_pages: u64) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "data": data,
            "meta": {"total_records": 3, "current_page": current, "total_pages": total_pages, "per_page": 100},
            "links": {}
        }))
    }

    #[tokio::test]
    async fn fetch_all_walks_every_page() {
        let server = MockServer::start().await;
        let user = Uuid::new_v4();
        let category = |name: &str| json!({"uuid": Uuid::new_v4(), "user_uuid": user, "name": name, "description": null});
        Mock::given(path("/categories"))
            .and(query_param("page", "1"))
            .and(query_param("user_uuid", user.to_string().as_str()))
            .respond_with(page(json!([category("a"), category("b")]), 1, 2))
            .mount(&server)
            .await;
        Mock::given(path("/categories"))
            .and(query_param("page", "2"))
            .respond_with(page(json!([category("c")]), 2, 2))
            .mount(&server)
            .await;
        let names: Vec<String> = client(&server)
            .list_categories("t", user)
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[tokio::test]
    async fn errors_carry_backend_status_and_message() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(409).set_body_json(
                json!({"error": "CONFLICT", "message": "Account has 3 transactions"}),
            ))
            .mount(&server)
            .await;
        match client(&server)
            .delete_account("t", Uuid::new_v4())
            .await
            .unwrap_err()
        {
            ApiError::Http {
                status,
                code,
                message,
            } => {
                assert_eq!(
                    (status, code.as_str(), message.as_str()),
                    (409, "CONFLICT", "Account has 3 transactions")
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn list_transactions_sends_filters_and_sort() {
        let server = MockServer::start().await;
        let user = Uuid::new_v4();
        let account = Uuid::new_v4();
        Mock::given(path("/transactions"))
            .and(query_param("sort", "-transaction_date"))
            .and(query_param("account_uuid", account.to_string().as_str()))
            .and(query_param("date_from", "2026-10-01"))
            .and(query_param("search", "cafe"))
            .and(query_param("per_page", "10"))
            .respond_with(page(json!([]), 1, 1))
            .expect(1)
            .mount(&server)
            .await;
        let query = TransactionQuery {
            account_uuid: Some(account),
            date_from: chrono::NaiveDate::from_ymd_opt(2026, 10, 1),
            search: Some("cafe".into()),
            ..Default::default()
        };
        let result = client(&server)
            .list_transactions("t", user, &query, 1, 10)
            .await
            .unwrap();
        assert!(result.data.is_empty());
    }

    #[tokio::test]
    async fn transport_errors_are_reported() {
        let api = ApiClient::new("http://127.0.0.1:9", Duration::from_secs(1)).unwrap();
        assert!(matches!(
            api.list_currencies("t").await,
            Err(ApiError::Transport(_))
        ));
    }
}
