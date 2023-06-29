use std::str::Split;
use std::convert::From;
use sequence_trie::SequenceTrie;
use tokio::{sync::mpsc, task::JoinHandle};
use tokio::sync::oneshot;
use tokio::sync::mpsc::error::SendError;
use log::{debug, error};
use thiserror::Error;

use crate::Label;

#[derive(Debug, Error)]
pub enum DBError{
    #[error("oneshot channel error")]
    BackChannel(#[from] oneshot::error::RecvError),
    #[error("database channel error")]
    DatabaseChannel(#[from] mpsc::error::SendError<DBRequest>),
}

#[derive(PartialEq, Debug)]
pub enum RequestError {
    InvalidTopic
}

#[derive(PartialEq, Debug)]
pub enum DBResult{
    None,
    Some(Label),
    Denied(RequestError),
}

impl From<Option<Label>> for DBResult {
    fn from(opt: Option<Label>) -> Self {
        match opt {
            Some(value) => Self::Some(value),
            None => Self::None,
        }
    }
}

struct SequenceIterator<'a> {
    data: &'a [&'a str],
    index: usize,
}

impl <'a>SequenceIterator<'a>  {
    fn new(data: &'a [&'a str]) -> Self {
        SequenceIterator {
            data,
            index: 0
        }

    }
}

impl<'a> Iterator for SequenceIterator<'a> {
    type Item = &'a[&'a str];

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.data.len() {
            return None;
        }
        if self.data[self.index] == "+" {
            let old = self.index;
            self.index += 1;
            return Some(&self.data[old..self.index])
        }

        let start = self.index;
        let mut end = self.index;

        while end < self.data.len() && self.data[end] != "+" {
            end += 1;
        }

        self.index = end;

        Some(&self.data[start..end])
    }
}

#[derive(Debug)]
pub struct TopicDB {
    trie: SequenceTrie<String, Label>,
}

impl<'s> TopicDB {
    pub fn new() -> Self{
        Self{
            trie: SequenceTrie::new(),
        }
    }

    fn split_topic<'topic>(topic: &'topic str) -> Split<'topic, char> {
        topic.split('/')
    }

    fn get_min(sub_trie: &&SequenceTrie<String, Label>) -> Option<Label>{
            return sub_trie.values().min().copied()
    }

    fn get_value(sub_trie: &&SequenceTrie<String, Label>) -> Option<Label>{
            sub_trie.value().copied()
    }

    pub fn insert(&mut self, topic:&str, label:Label) -> Option<Label> {
        let keys = Self::split_topic(topic);
        self.trie.insert(keys, label)
    }

    pub fn get(&'s self, topic: &str) -> DBResult {
        if topic == "#" {
            return self.trie.values().min().copied().into()
        }

        let (keys, wildcard) = if topic.ends_with("/#") {
            let last_index = topic.len() - 2; // len has to be at least one since it end with a "/#"
            let topic_rest = &topic[..last_index];
            if topic_rest.contains('#') {
                return DBResult::Denied(RequestError::InvalidTopic)
            }
            (Self::split_topic(topic_rest).collect::<Vec<&str>>(), true)
        }else {
            (Self::split_topic(topic).collect(), false)
        };
        
        let mut nodes = vec![&self.trie];
        for sub_key in SequenceIterator::new(&keys) {
            let mut new_nodes: Vec<&'s SequenceTrie<String, Label>> = Vec::new();
            for n in &nodes {
                if sub_key == ["+"] {
                    new_nodes.append(n.children().as_mut());
                } else {
                    match n.get_node(sub_key.iter().map(|x| *x)) {
                        None => {},
                        Some(node) => {
                            new_nodes.push(node);
                        }
                    }
                }
            }
            nodes.clear();
            nodes.append(&mut new_nodes);
        }

        let nodes = nodes.iter();
        let min_label = if wildcard {
            nodes.filter_map(Self::get_min).min()
        }
        else  {
            nodes.filter_map(Self::get_value).min()
        };

        min_label.into()
    }
}


#[derive(Debug)]
pub enum DBRequest{
    Get(String, oneshot::Sender<DBResult>),
    Insert(String, Label)
}

#[derive(Clone)]
pub struct Database {
    tx: mpsc::Sender<DBRequest>,
}

impl Database {
    pub fn new() -> (Database, JoinHandle<()>){
        let (tx, mut rx) = mpsc::channel::<DBRequest>(3200);
        let handle = tokio::spawn(async move{
            let mut database = TopicDB::new();
            loop {
                let msg = rx.recv().await;  
                match msg {
                    Some(DBRequest::Insert(topic, label)) => {
                        database.insert(&topic, label);
                    },
                    Some(DBRequest::Get(topic, reply_channel)) => {
                        let result = database.get(&topic);
                        match reply_channel.send(result){
                            Ok(()) => {
                                debug!("Send reply back to requester")
                            },
                            Err(e) => {
                                error!("Was not able to reply on one shoot channel {e:?}")
                            },
                        };
                    }
                    None => {
                        dbg!("none");
                    }
                }
            }
        });
        let db = Database {
            tx
        };
        (db, handle)
    }
    pub async fn insert(&self, topic:String,  label:Label) -> Result<(), SendError<DBRequest>>{
        let msg = DBRequest::Insert(topic, label);
        self.tx.send(msg).await
    }
    pub async fn get(&self, topic:String) -> Result<DBResult, DBError>{
        let (tx, rx) = oneshot::channel::<DBResult>();
        let msg = DBRequest::Get(topic, tx);
        self.tx.send(msg).await?;
        let label = rx.await?;
        Ok(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insert() {
        let mut db = TopicDB::new();
        db.insert("test/test", 5);
        db.insert("test/abc", 5);

        assert_eq!(db.get("test/test"), DBResult::Some(5));
    }
    #[test]
    fn fail_get() {
        let mut db = TopicDB::new();
        db.insert("test/test", 5);
        assert_eq!(db.get("test"), DBResult::None);
    }
    #[test]
    fn insert_start_slash() {
        let mut db = TopicDB::new();
        db.insert("/test", 666);
        assert_eq!(db.get("/test"), DBResult::Some(666));
    }
    #[test]
    fn insert_double_slash() {
        let mut db = TopicDB::new();
        db.insert("lol//test", 666);
        assert_eq!(db.get("lol//test"), DBResult::Some(666));
    }
    #[test]
    fn wildcard() {
        let mut db = TopicDB::new();
        db.insert("in/test", 5);
        db.insert("in/abc", 4);
        db.insert("in/test/abc", 9);
        db.insert("out/abc", 1);

        assert_eq!(db.get("in/#"), DBResult::Some(4));
    }
    #[test]
    fn solewildcard() {
        let mut db = TopicDB::new();
        db.insert("test/test", 5);
        db.insert("test/abc", 3);
        db.insert("in/test", 2);
        db.insert("in/abc", 9);
        db.insert("in/test/abc", 9);
        db.insert("out/abc", 1);
        db.insert("zero/abc/zero", 0);

        assert_eq!(db.get("#"), DBResult::Some(0));
    }
    #[test]
    fn single_level_wildcard() {
        let mut db = TopicDB::new();
        db.insert("test/test", 3);
        db.insert("test/abc", 3);
        db.insert("in/2/test/test", 6);
        db.insert("in/2/abc/test", 9);
        db.insert("in/test/abc", 1);
        db.insert("out/abc", 1);
        db.insert("zero/abc/zero", 0);

        assert_eq!(db.get("in/2/+/test"), DBResult::Some(6));
    }
}
