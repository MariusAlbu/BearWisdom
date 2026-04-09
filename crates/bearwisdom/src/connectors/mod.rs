pub mod connector_db;
pub mod matcher;
pub mod registry;
pub mod traits;
pub mod types;

// --- New-architecture connector implementations ---
pub mod graphql_connector;
pub mod grpc_connector;
pub mod ipc_connector;
pub mod mq_connector;
pub mod rest_connector;
pub mod di_connector;

pub mod django;
pub mod docker_compose;
pub mod dockerfile;
pub mod kubernetes;
pub mod dotnet_http_client;
pub mod ef_core;
pub mod electron_ipc;
pub mod fastapi_routes;
pub mod frontend_http;
pub mod graphql;
pub mod grpc;
pub mod http_api;
pub mod message_queue;
pub mod react_patterns;
pub mod tauri_ipc;
