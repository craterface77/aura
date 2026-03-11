pub mod proto {
    tonic::include_proto!("aura_l2");
}

mod service;
pub use service::AuraL2Service;
