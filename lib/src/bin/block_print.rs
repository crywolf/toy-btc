use std::io::Error;

use btclib::blockchain::Block;
use btclib::Serializable;

fn main() -> Result<(), Error> {
    let path = if let Some(arg) = std::env::args().nth(1) {
        arg
    } else {
        eprintln!("Usage: block_print <block_file>");
        std::process::exit(1);
    };

    let file = std::fs::File::open(path)?;
    let block = Block::load(file)?;
    println!("{:#?}", block);

    Ok(())
}
