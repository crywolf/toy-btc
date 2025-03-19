use argh::FromArgs;

#[derive(FromArgs)]
/// A toy bitcoin node
pub struct Args {
    #[argh(option, default = "String::from(\"127.0.0.1\")")]
    /// host address (default: 127.0.0.1)
    pub host: String,
    #[argh(option)]
    /// port number
    pub port: u16,
    #[argh(option, default = "String::from(\"./blockchain.cbor\")")]
    /// blockchain file location (default: ./blockchain.cbor)
    pub blockchain_file: String,
    #[argh(positional)]
    /// addresses of initial nodes
    pub nodes: Vec<String>,
}

pub fn args() -> Args {
    argh::from_env()
}
