use std::process::exit;

use btclib::{blockchain::Block, Saveable};

fn main() {
    // parse block path and steps count from the first and second argument respectively
    let mut args = std::env::args();
    let (path, steps) = if let (Some(arg1), Some(arg2)) = (args.nth(1), args.next()) {
        (arg1, arg2)
    } else {
        eprintln!("Usage: miner <block_file> <steps>");
        exit(1);
    };
    // parse steps count
    let steps: usize = if let Ok(s @ 1..=usize::MAX) = steps.parse() {
        s
    } else {
        eprintln!("<steps> should be a positive integer");
        exit(1);
    };

    let orig_block = Block::load_from_file(path).expect("Failed to load block");
    let mut block = orig_block.clone();

    while !block.header.mine(steps) {
        println!("mining...");
    }

    // print original block and its hash
    println!("original: {:#?}", orig_block);
    println!("hash: {}", orig_block.header.hash());

    // print mined block and its hash
    println!("-------------------------");
    println!("final: {:#?}", block);
    println!("hash: {}", block.header.hash());
}
