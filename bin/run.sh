#!/bin/sh
tmux split-window -v -b "$PWD/bin/subscriber.sh '#' 20|| read"
tmux split-window -v -b "cargo run --bin proxy || read"
tmux split-window -v -b "cargo run --bin label_db || read"
tmux split-window -v -b -l 30 "$PWD/bin/broker.sh 11883||read"
tmux split-window -h -b "$PWD/bin/broker.sh 21883||read"
