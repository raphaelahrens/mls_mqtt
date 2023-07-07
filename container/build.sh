#!/bin/sh
#
#
pwd

cargo build --release

cp /mls/target/release/proxy /mls/release/
cp /mls/target/release/label_db /mls/release/
