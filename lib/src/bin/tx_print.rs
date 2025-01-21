use std::io::Error;

use btclib::blockchain::Tx;
use btclib::Saveable;

fn main() -> Result<(), Error> {
    let path = if let Some(arg) = std::env::args().nth(1) {
        arg
    } else {
        eprintln!("Usage: tx_print <tx_file>");
        std::process::exit(1);
    };

    let file = std::fs::File::open(path)?;
    let tx = Tx::load(file)?;
    println!("{:#?}", tx);

    Ok(())
}
