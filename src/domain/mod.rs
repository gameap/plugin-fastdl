//! FastDL installation state, configuration, and API response models.

pub mod node;
pub mod server;
pub mod validation;

pub use node::{
    DaemonConfig, NodeConfig, NodeOs, NodeResponse, NodeSetupStatus, NodesResponse, SetupInput,
    SetupStatus,
};
pub use server::{
    ConfigureResponse, Engine, ServerDropIn, ServerInput, ServerResponse, ServerState,
};
pub use validation::{validate_relative, validate_url};
