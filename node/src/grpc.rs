pub mod miner_api;
pub mod node_api;
pub mod peers;
pub mod wallet_api;

pub(crate) const FILE_DESCRIPTOR_SET: &[u8] =
    tonic::include_file_descriptor_set!("reflection_descriptor");

pub mod node {
    pub mod pb {
        tonic::include_proto!("node");
    }
}

pub mod miner {
    pub mod pb {
        tonic::include_proto!("miner");
    }
}

pub mod wallet {
    pub mod pb {
        tonic::include_proto!("wallet");
    }
}

fn log_error(e: impl std::fmt::Debug) {
    eprintln!("Error: {:?}", e);
}
