#!/usr/bin/env python3

import sys
from pathlib import Path
import paho.mqtt.client as mqtt
from cbor2 import loads

from base64 import b64decode

import nacl
from nacl.signing import VerifyKey

from nacl.encoding import RawEncoder

public_key_signature = b'\x00\x00\x00\x20'


# Extract length bytes counting from the first occurence of the given signature.
def bytes_after(signature, length, bytestr):
    start = bytestr.find(signature) + len(signature)
    return bytestr[start:start+length]


def extract_pub_ed25519(pub_key_file):
    with pub_key_file.open() as fd:
        pub_key_line = fd.readline()
        pub_key_parts = pub_key_line.split(' ')
        key_type = pub_key_parts[0]  # line = "ssh-ed25519 <hex_key> <comment>"
        if key_type != "ssh-ed25519":
            return None
        b64_pub_key = pub_key_parts[1]  # line = "ssh-ed25519 <hex_key> <comment>"
        openssh_pub_bytes = b64decode(b64_pub_key)
        pub_bytes = bytes_after(public_key_signature,
                                32,
                                openssh_pub_bytes)
        nacl_pub_ed = VerifyKey(key=pub_bytes, encoder=RawEncoder)
        return nacl_pub_ed
    return None

# The callback for when the client receives a CONNACK response from the server.


def on_connect(client, userdata, flags, rc):
    print("Connected with result code "+str(rc))

    # Subscribing in on_connect() means that if we lose the connection and
    # reconnect then subscriptions will be renewed.
    client.subscribe(sys.argv[3])

# The callback for when a PUBLISH message is received from the server.

def build_on_message(pub_keys):
    def on_message(client, userdata, msg):
        print(msg.topic)
        cbor_load = loads(msg.payload)
        msg_bytes = bytes(cbor_load["payload"] + cbor_load["ad"]) + cbor_load["datetime"].to_bytes(8, 'big') + cbor_load["key_id"].encode()
        try:
            pub_key = pub_keys[cbor_load["key_id"]]
            print(pub_key.verify(msg_bytes, bytes(cbor_load["signature"])))
        except nacl.exceptions.BadSignatureError as e:
            print(e)
            print(msg_bytes)

        print()
    return on_message


pub_keys = {
        'proxy.label.1': extract_pub_ed25519(Path("./data/label.key.pub")),
        'proxy.info.1': extract_pub_ed25519(Path("./data/info.key.pub")),
        }


client = mqtt.Client()
client.on_connect = on_connect
client.on_message = build_on_message(pub_keys)

client.connect(sys.argv[1], int(sys.argv[2]), 60)

# Blocking call that processes network traffic, dispatches callbacks and
# handles reconnecting.
# Other loop*() functions are available that give a threaded interface and a
# manual interface.
client.loop_forever()
