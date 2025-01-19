use btclib::blockchain::Tx;
use btclib::Saveable;

fn main() {
    let path = if let Some(arg) = std::env::args().nth(1) {
        arg
    } else {
        eprintln!("Usage: tx_print <tx_file>");
        std::process::exit(1);
    };

    if let Ok(file) = std::fs::File::open(path) {
        let tx = Tx::load(file).expect("Failed to load transaction");
        println!("{:#?}", tx);
    }
}
