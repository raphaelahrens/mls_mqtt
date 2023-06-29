use std::{path::PathBuf, sync::Arc, time::Duration};
use std::time::SystemTime;
use std::str::FromStr;
use std::io;

use eyre::{eyre, Result};
use log::{debug, error, info};
use rumqttc::{
    AsyncClient, ConnectionError,
    Event::{Incoming, Outgoing},
    MqttOptions,
    Packet,
    QoS,
    Publish,
};
use serde::{Deserialize, Serialize};
use tokio::{net::UnixStream, runtime::Builder, select, task};
use tokio::net::UnixListener;


use mls::{
    ErrorCounter,
    LabeledInfo,
    SignedMsg,
    PublicKey,
    topicdb::Database,
    topicdb::DBResult,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfPubKey {
    key: String,
    id: String
}

impl ConfPubKey {
    fn get_key(&self) -> Result<PublicKey> {
        let ssh_pubkey = ssh_key::PublicKey::from_openssh(&self.key)?;
        let pub_key = ssh_pubkey.key_data();
        let pub_key = match pub_key {
            ssh_key::public::KeyData::Ed25519(key_data) => {
                key_data.clone().try_into()?
            }
            _ => {
                return Err(eyre!("Key is not an Ed25519"))
            }
        };
        Ok(PublicKey::new(
            pub_key,
        ))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    broker: String,
    log_level: String,
    mls_topic: String,
    mls_pubkey: ConfPubKey,
    threads: usize,
    socket_path: PathBuf,
}

impl ::std::default::Default for Config {
    fn default() -> Self {
        Self {
            broker: "mqtt://localhost:1883?client_id=label_db1".into(),
            log_level: "info".into(),
            mls_topic: "mls/info".into(),
            mls_pubkey: ConfPubKey{
                key: "<type> <public_key>[<comment>]".into(),
                id: "proxy.info.1".into(),
            },
            threads: 2,
            socket_path: "/tmp/labeldb.sock".into(),
        }
    }
}

fn setup_logger(level: &str) -> Result<()> {
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339_seconds(SystemTime::now()),
                record.level(),
                record.target(),
                message
            ))
        })
        .level(log::LevelFilter::from_str(level)?)
        .chain(std::io::stdout())
        .apply()?;
    Ok(())
}

fn main() -> Result<()> {
    let cfg: Config = confy::load("mls", "labeldb.conf")?;
    setup_logger(&cfg.log_level)?;
    Builder::new_multi_thread()
        .enable_all()
        .worker_threads(cfg.threads)
        .build()?
        .block_on(async move { main_loop(cfg).await })?;
    Ok(())
}

async fn main_loop(cfg: Config) -> Result<()> {
    let (db, db_handle) = Database::new();
    let verify_key = Arc::new(cfg.mls_pubkey.get_key()?);
    let broker_handle = task::spawn(broker_task(cfg.broker.clone(), cfg.mls_topic.clone(), verify_key.clone(), db.clone()));
    let socket_handle = task::spawn(socket_task(cfg.socket_path.clone(), db.clone()));
    select! {
        e = broker_handle => {
            e??;
        },
        e = socket_handle => {
            e??;
        },
        e = db_handle => {
            e?;
        }
    }
    Ok(())
}

async fn handle_get(topic:&[u8], db: &Database, stream: &UnixStream) -> Result<()>{
    let topic = std::str::from_utf8(topic)?;
    let label = db.get(topic.to_string()).await?;
    let lable_str;
    let reply = match label {
        DBResult::None => "None",
        DBResult::Some(label) =>{
            lable_str = label.to_string();
            &lable_str
        },
        DBResult::Denied(_) => "Denied"
    };
    stream.try_write(&reply.as_bytes())?;
    stream.try_write(&"\n".as_bytes())?;
    Ok(())
}

async fn handle_request(stream: UnixStream, db: Database) -> Result<()> {
    loop {
        // Wait for the socket to be readable
        stream.readable().await?;

        // Creating the buffer **after** the `await` prevents it from
        // being stored in the async task.
        let mut cmd_buf = [0; 4+65535+1];

        // Try to read data, this may still fail with `WouldBlock`
        // if the readiness event is a false positive.
        match stream.try_read(&mut cmd_buf) {
            Ok(0) => break,
            Ok(n) => {
                if &cmd_buf[0..4] == b"GET " {
                    let topic = &cmd_buf[4..n-1];
                    handle_get(&topic, &db, &stream).await?;
                } else {
                    error!("Unknown command");
                    return Err(eyre!("Unknown command"))
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                continue;
            }
            Err(e) => {
                error!("Error");
                return Err(e.into());
            }
        }
    }
    Ok(())
}

async fn socket_task(path: PathBuf, db:Database) -> Result<()> {
    let listener = UnixListener::bind(path)?;
    loop {
        let db_clone = db.clone();
        match listener.accept().await {
            Ok((stream, _addr)) => {
                task::spawn(async move {
                    match handle_request(stream, db_clone).await {
                        Ok(()) => {
                        },
                        Err(e) => {
                            error!("Request handle failed {e:?}");
                        }
                    }
                });
            }
            Err(e) => { 
                break Err(e.into());
            }
        }
    }
}

async fn handle_topic_info(db:Database, verify_key:Arc<PublicKey>, msg: Publish) -> Result<()> {
    debug!("Processing Incoming message = {:?}", msg);
    match ciborium::de::from_reader::<SignedMsg, &[u8]>(&msg.payload[..]){
        Err(e) => {
            error!("Error = {e}")
        },
        Ok(msg) => {
            let signed_msg = match msg.verify(&verify_key){
                Ok(verified_msg) => verified_msg,
                Err(e) => {
                    error!("Signature verification failed. Error = {e}");
                    return Err(e.into());
                }
            };
            match ciborium::de::from_reader::<LabeledInfo, &[u8]>(signed_msg) {
                Err(e) => {
                    error!("Error = {e}")
                },
                Ok(topic_info) => {
                    debug!("Inserting {topic_info:?}");
                    db.insert(topic_info.topic, topic_info.label).await?;
                }
            }
        },
    }
    Ok(())
}


async fn broker_task(broker: String, topic: String, verify_key: Arc<PublicKey>, db: Database) -> Result<()> {
    let mut error_broker = ErrorCounter::new();
    info!("broker = {}", broker);
    let mut broker_mqttoptions = MqttOptions::parse_url(broker)?;
    broker_mqttoptions.set_keep_alive(Duration::from_secs(30));

    let (broker, mut broker_eventloop) = AsyncClient::new(broker_mqttoptions, 10);
    loop {
        match broker_eventloop.poll().await {
            Ok(notification) => {
                match notification {
                    Incoming(Packet::Publish(msg)) => {
                        task::spawn(handle_topic_info(db.clone(), verify_key.clone(), msg));
                    }
                    Incoming(Packet::ConnAck(_)) => {
                        error_broker.reset();
                        broker.subscribe(topic.clone(), QoS::AtMostOnce).await?;
                    }
                    Incoming(incoming) => {
                        debug!("Received Incoming event = {:?}", incoming);
                    }
                    Outgoing(outgoing) => {
                        debug!("Received Outgoing event = {:?}", outgoing);
                    }
                };
            }
            Err(err) => {
                error_broker.inc();
                match err {
                    ConnectionError::MqttState(_) => {}
                    ConnectionError::FlushTimeout
                    | ConnectionError::Tls(_)
                    | ConnectionError::NotConnAck(_)
                    | ConnectionError::RequestsDone => {
                        error!("{:?}", err);
                    }
                    ConnectionError::Io(_)
                    | ConnectionError::NetworkTimeout
                    | ConnectionError::ConnectionRefused(_) => {
                        tokio::time::sleep(Duration::from_secs(3)).await;
                    }
                }
            }
        }
        if error_broker.is_too_mutch() {
            break Err(eyre!("Broker connection exeded the error limit"));
        }
    }
}
