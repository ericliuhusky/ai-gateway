pub mod handlers;
pub mod routes;

pub use handlers::AppState;
pub use routes::build_router;

#[derive(Clone, Copy, Debug)]
pub struct RequestScope {
    pub owner_user_id: Option<i64>,
}
