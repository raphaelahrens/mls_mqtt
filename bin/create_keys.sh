#!/bin/sh

directory="$1"

if [ -d "$directory" ]; then
    ssh-keygen -f "$directory/info.key" -t ed25519 -N ""
    ssh-keygen -f "$directory/label.key" -t ed25519 -N ""
    exit 0
else
    printf "Error: %s is not directory \n" "$directory"
    exit 1
fi
