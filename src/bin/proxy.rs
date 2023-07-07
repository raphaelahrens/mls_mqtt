use clap::Parser;
use eyre::{eyre, Result};
use log::{debug, error, info};
use rumqttc::{ AsyncClient, ConnectionError, Event::{Incoming, Outgoing}, MqttOptions, Packet::{ConnAck, Publish}, QoS,};
use std::str::FromStr;
use std::time::Duration;
use std::time::SystemTime;
use std::{collections::HashMap, path::{Path, PathBuf}};
use tokio::runtime::Builder;
use tokio::task;

use serde::{Deserialize, Serialize};

use mls::{ErrorCounter, Label, LabeledInfo, Key};

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfKey {
    path: PathBuf,
    id: String
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Config {
    source: String,
    sink: String,
    log_level: String,
    mls_topic: String,
    topics: HashMap<String, Label>,
    label_key: ConfKey,
    info_key: ConfKey,
}

impl ::std::default::Default for Config {
    fn default() -> Self {
        Self {
            source: "mqtt://localhost:1883?client_id=1".into(),
            sink: "mqtt://localhost:1883?client_id=1".into(),
            log_level: "info".into(),
            mls_topic: "mls/info".into(),
            topics: HashMap::new(),
            label_key: ConfKey{
                id: "proxy.label.1".into(),
                path: "./data/label.key".into(),
            },
            info_key: ConfKey{
                id: "proxy.info.1".into(),
                path: "./data/info.key".into(),
            }
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
struct AdditionalData {
    label: Label,
}

impl AdditionalData {
    fn new(label: Label) -> Self{
        AdditionalData {
            label,
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

fn get_key(conf: &ConfKey) -> Result<Key>{
    let ssh_secretkey = ssh_key::PrivateKey::read_openssh_file(&conf.path)?;
    let secret = ssh_secretkey.key_data();
    let secret = match secret {
        ssh_key::private::KeypairData::Ed25519(key_pair) => {
            key_pair.private.clone().into()
        }
        _ => {
            return Err(eyre!("Key is not an Ed25519"))
        }
    };
    Ok(Key::new(
        secret,
        conf.id.clone()
    ))
}

fn main() -> Result<()> {
    let args = Args::parse();
    dbg!(&args);
    let home_conf = dbg!(confy::get_configuration_file_path("mls", "proxy.conf"));
    let mut conf_path = Path::new("/usr/local/etc/mls/proxy.conf");
    
    if let Some(ref path) = args.config {
        // If the --config flag has been give use that path
        conf_path = &path
    } else if let Ok(ref path) = home_conf{
        // If the --config flag has been give use that path
        conf_path = &path
    }
    dbg!(&conf_path);
    let cfg:Config = confy::load_path(&conf_path)?;
    
    if &cfg.label_key.id == &cfg.info_key.id {
        return Err(eyre!("The ids of the label and info key are the same"));
    }
    setup_logger(&cfg.log_level)?;
    
    Builder::new_multi_thread()
        .enable_all()
        .worker_threads(4)
        .build()?
        .block_on(async move { main_loop(cfg).await })?;
    Ok(())
}

fn label_msg(
    topic: &str,
    payload: &[u8],
    label_map: &HashMap<String, Label>,
    label_key: &Key,
    info_key: &Key,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let Some(label) = label_map.get(topic) else { todo!();};
    let ad = AdditionalData::new(*label);
    let mut buffer: Vec<u8> = Vec::with_capacity(payload.len() * 8);
    let mut ad_buf: Vec<u8> = Vec::with_capacity(4098);
    ciborium::ser::into_writer(&ad, &mut ad_buf)?;
    let label_msg = label_key.sign_with_ad(payload.to_vec(), ad_buf);
    ciborium::ser::into_writer(&label_msg, &mut buffer)?;

    let mut info_buf: Vec<u8> = Vec::with_capacity(4098);
    let label_info = LabeledInfo::new(topic, *label);
    let info_msg = info_key.sign(label_info.serialize()?.to_vec());
    ciborium::ser::into_writer(&info_msg, &mut info_buf)?;
    Ok((buffer, info_buf))
}


async fn source_loop(
    eventloop: &mut rumqttc::EventLoop,
    source:AsyncClient,
    sink:AsyncClient,
    label_key:Key,
    info_key:Key,
    cfg:Config) -> Result<()> {
    let mut error_source = ErrorCounter::new();
    let filters = cfg
        .topics
        .keys()
        .map(|topic| rumqttc::SubscribeFilter::new(topic.clone(), QoS::AtMostOnce));
    loop {
        debug!("source loop");
        match eventloop.poll().await {
           Ok(notification) => {
            match notification {
                Incoming(Publish(msg)) => {
                        debug!("Foward Incoming message = {:?}", msg);
                        let (labeled_payload, label_info) = label_msg(&msg.topic, &msg.payload, &cfg.topics, &label_key, &info_key)?;
                        sink.publish(msg.topic, msg.qos, msg.retain, labeled_payload).await?;
                        sink.publish(&cfg.mls_topic, QoS::ExactlyOnce, false, label_info).await?;
                },
                Incoming(ConnAck(_)) => {
                    info!("subscribe to source");
                    error_source.reset();
                    source.subscribe_many(filters.clone()).await?;
                },
                Incoming(incoming) => {
                    debug!("Source Received Incoming event = {:?}", incoming);
                },
                Outgoing(outgoing) => {
                    debug!("Source Received Outgoing event = {:?}", outgoing);
                },
            }
           }
           Err(e) => {
                delay_on_disconnect(e).await;
                error_source.inc();
                if error_source.is_too_mutch() {
                    return Err(eyre!("Source connection exeded the error limit"));
                }
           }
       }
    }
}


async fn sink_loop(eventloop: &mut rumqttc::EventLoop) -> Result<()> {
    let mut error_sink = ErrorCounter::new();
    loop {
        debug!("sink loop");
        match eventloop.poll().await {
           Ok(notification) => {
                match notification {
                    Incoming(ConnAck(_)) => {
                        info!("reconnected sink");
                        error_sink.reset();
                    },
                    Incoming(incoming) => {
                        debug!("Sink Received Incoming event = {:?}", incoming);
                    },
                    Outgoing(outgoing) => {
                        debug!("Sink Received Outgoing event = {:?}", outgoing);
                    },
                };
           }
           Err(e) => {
                error_sink.inc();
                delay_on_disconnect(e).await;
                if error_sink.is_too_mutch() {
                    return Err(eyre!("Sink connection exeded the error limit"));
                }
           }
        }
    }
}

async fn delay_on_disconnect(err: ConnectionError) {
        debug!("{:?}", err);
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

async fn main_loop(cfg: Config) -> Result<()> {
    info!("source = {}", cfg.source);
    info!("sink = {}", cfg.sink);

    let label_key = get_key(&cfg.label_key)?;
    let info_key = get_key(&cfg.info_key)?;

    
    let mut source_mqttoptions = MqttOptions::parse_url(cfg.source.clone())?;
    source_mqttoptions.set_keep_alive(Duration::from_secs(30));
    let mut sink_mqttoptions = MqttOptions::parse_url(cfg.sink.clone())?;
    sink_mqttoptions.set_keep_alive(Duration::from_secs(30));

    let (source, mut source_eventloop) = AsyncClient::new(source_mqttoptions, 10);
    let (sink, mut sink_eventloop) = AsyncClient::new(sink_mqttoptions, 10);

    let source_handle = task::spawn(async move  {
        source_loop(&mut source_eventloop, source, sink, label_key, info_key, cfg).await
    });
    let sink_handle = task::spawn(async move  {
        sink_loop(&mut sink_eventloop).await
    });
    debug!("task started");
    tokio::select! {
       result = source_handle => {
            debug!("got source");
           result??;
       }
       result = sink_handle => {
            debug!("got sink");
           result??;
       }
    }
    Ok(())
}
