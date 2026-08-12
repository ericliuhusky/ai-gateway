pub mod handlers;
pub mod routes;

pub use handlers::AppState;
pub use routes::{build_router, build_router_with_web};

#[derive(Clone, Copy, Debug)]
pub struct RequestScope {
    pub owner_user_id: Option<i64>,
}
