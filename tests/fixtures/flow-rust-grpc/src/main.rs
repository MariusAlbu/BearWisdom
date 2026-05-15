use axum::{Router, routing::get};

mod schema {
    pub mod users { pub struct table; }
}

pub async fn run() {
    // Axum route — Consumer HttpCall.
    let app: Router<()> = Router::new()
        .route("/api/users", get(handler))
        .route("/api/items/:id", get(handler));

    // reqwest Producer HttpCall.
    let client = reqwest::Client::new();
    let _ = client.get("/external/api/v1/data").send().await;
    let _ = client.post("https://api.example.com/items").send().await;

    // SQLx — DbQuery.
    let pool = sqlx::SqlitePool::connect("").await.unwrap();
    let _ = sqlx::query!("SELECT id FROM users WHERE active = 1").fetch_all(&pool).await;
    let _ = sqlx::query_as!(User, "SELECT id, name FROM users").fetch_all(&pool).await;
    let _ = sqlx::query!("INSERT INTO items (a) VALUES (1)").execute(&pool).await;

    // Diesel — DbQuery.
    let _ = schema::users::table
        .filter(schema::users::id.eq(1))
        .first::<User>(&conn);

    // Tonic — RpcCall.
    let response = UserServiceClient::new(channel)
        .get_user(request)
        .await;
    let response2 = HelloServiceClient::connect("http://localhost:50051")
        .say_hello(request)
        .await;
}

async fn handler() -> &'static str { "ok" }

struct User { id: i64, name: String }
struct UserServiceClient;
impl UserServiceClient {
    pub fn new<T>(_t: T) -> Self { Self }
    pub async fn get_user(self, _req: ()) -> Result<(), ()> { Ok(()) }
}
struct HelloServiceClient;
impl HelloServiceClient {
    pub fn connect<T>(_t: T) -> Self { Self }
    pub async fn say_hello(self, _req: ()) -> Result<(), ()> { Ok(()) }
}
