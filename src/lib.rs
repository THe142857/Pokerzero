pub mod app;
pub mod app_config;
pub mod models;
pub mod schema;

use actix_session::Session;
use diesel::r2d2::ConnectionManager;
use diesel::sqlite::SqliteConnection;
use dotenvy::dotenv;
use r2d2::Pool;
use serde::{Deserialize, Serialize};
use std::env;

use lazy_static::lazy_static;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct UserData {
    pub email: String,
    pub displayName: String,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TeamData {
    pub name: String,
    pub members: Vec<UserData>,
}

// Build a database connection pool for server functions
lazy_static! {
    pub static ref DB_CONNECTION: Pool<ConnectionManager<SqliteConnection>> = {
        dotenv().ok();
        let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        Pool::builder()
            .max_size(15)
            .build(ConnectionManager::<SqliteConnection>::new(database_url))
            .unwrap()
    };
}

pub fn get_azure_secret() -> String {
    env::var("AZURE_SECRET").expect("AZURE_SECRET must be set in .env")
}
