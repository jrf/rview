build:
    cargo build

release:
    cargo build --release

install:
    cargo install --path .

run *args:
    cargo run -- {{args}}

clean:
    cargo clean
