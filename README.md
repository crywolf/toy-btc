# toy-btc


Toy bitcoin protocol implementation consisting of a bitcoin **node**, **miner** and **wallet**.

Inspired by book [Building bitcoin in Rust](https://braiins.com/books/building-bitcoin-in-rust) but with enhancements and optimizations:

* gRPC communication (besides legacy TCP messages exchange)
* Nodes subscribe to each other thus creating real bi-directional network
* Nodes will reconnect if conection to other node is lost
* Periodical blockchain synchronization
* Graceful shutdown
* Docker compose for easy deployment and testing
* and more...


## Usage

### Create private and public keys for wallets

```shell
 cargo run --bin key_gen alice
```
will create Alice's public key `alice.pub.pem` and private key `alice.priv.cbor`.

```shell
cargo run --bin key_gen bob
```
will create Bob's public key `bob.pub.pem` and private key `bob.priv.cbor`


### Start in gRPC mode

#### using Docker compose

```shell
docker compose up --build
```
After the first run, the containers are built and you can omit the `--build` flag

#### mannually

_Nodes_:
```shell
cargo run --bin node-grpc -- --port 50001

cargo run --bin node-grpc -- --port 50002 --blockchain-file blockchain1.cbor 127.0.0.1:50001

cargo run --bin node-grpc -- --port 50003 --blockchain-file blockchain2.cbor 127.0.0.1:50002

cargo run --bin node-grpc -- --port 50004 --blockchain-file blockchain3.cbor 127.0.0.1:50003

cargo run --bin node-grpc -- --port 50005 --blockchain-file blockchain4.cbor 127.0.0.1:50004
```
Use `cargo run --bin node-grpc -- --help` for more usage info.


_Miners_:
```shell
cargo run --bin miner-grpc -- -a 127.0.0.1:50001 -p alice.pub.pem

cargo run --bin miner-grpc -- -a 127.0.0.1:50003 -p bob.pub.pem
```
Use `cargo run --bin miner-grpc -- --help` for more usage info.

### Create wallets config files
```shell
cargo run --bin good-wallet -- generate-config
```

Rename created wallet config TOML file and fill in info similar to this:
```toml
my_keys = [["alice.pub.pem", "alice.priv.cbor"]]
default_node = "127.0.0.1:50001"

[[contacts]]
name = "Alice"
key_path = "alice.pub.pem"

[[contacts]]
name = "Bob"
key_path = "bob.pub.pem"

[fee_config]
fee_type = "Percent"
value = 0.1
```

_Wallet_ can be started using this command (if you omit the `--cli` flag it will run in TUI mode):
```shell
cargo run --bin good-wallet -- --config .wallets/wallet_alice.toml --grpc --cli

cargo run --bin good-wallet -- --config .wallets/wallet_bob.toml --grpc --cli
```
Use `cargo run --bin good-wallet -- --help` for more usage info.


### Start in TCP mode

#### using Docker compose

```shell
docker compose -f compose_tcp.yaml up --build
```

#### mannually

_Nodes_:
```shell
cargo run --bin node -- --port 9000

cargo run --bin node -- --port 9001 --blockchain-file blockchain1.cbor localhost:9000

cargo run --bin node -- --port 9002 --blockchain-file blockchain2.cbor localhost:9001

cargo run --bin node -- --port 9003 --blockchain-file blockchain3.cbor localhost:9002

cargo run --bin node -- --port 9004 --blockchain-file blockchain4.cbor localhost:9003
```

_Miner_:
```shell
cargo run --bin miner -- -a localhost:9002 -p alice.pub.pem
```

_Wallets_:

The same usage as for gRPC above, just omit the `--grpc` flag.
