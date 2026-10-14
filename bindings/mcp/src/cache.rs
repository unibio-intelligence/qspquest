use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};

type Document = Arc<qspquest_modeling::document::CompiledDocument>;
type Compilation = Result<Document, Value>;
type Flight = Mutex<Option<Compilation>>;

#[derive(Default)]
pub(crate) struct CompilationFlights(Mutex<HashMap<String, Weak<Flight>>>);

fn compiled_hit(
    entries: &Mutex<VecDeque<(String, Document)>>,
    key: &str,
) -> Result<Option<Document>, Value> {
    let mut entries = entries.lock().map_err(|_| {
        serde_json::json!({"code":"compile_cache","message":"Compiled model cache is unavailable; restart the server"})
    })?;
    let Some(index) = entries.iter().position(|(name, _)| name == key) else {
        return Ok(None);
    };
    let entry = entries.remove(index).unwrap();
    let document = entry.1.clone();
    entries.push_back(entry);
    Ok(Some(document))
}

pub(crate) fn compiled(
    entries: &Mutex<VecDeque<(String, Document)>>,
    flights: &CompilationFlights,
    key: String,
    compile: impl FnOnce() -> Compilation,
) -> Compilation {
    if let Some(document) = compiled_hit(entries, &key)? {
        return Ok(document);
    }
    let flight = {
        let mut flights = flights.0.lock().map_err(|_| {
            serde_json::json!({"code":"compile_cache","message":"Compilation coordination is unavailable; restart the server"})
        })?;
        // Completed requests leave only weak references. Keep coordination
        // bounded by live callers rather than every model ever requested.
        flights.retain(|_, flight| flight.strong_count() > 0);
        if let Some(flight) = flights.get(&key).and_then(Weak::upgrade) {
            flight
        } else {
            let flight = Arc::new(Mutex::new(None));
            flights.insert(key.clone(), Arc::downgrade(&flight));
            flight
        }
    };
    let mut result = flight.lock().map_err(|_| {
        serde_json::json!({"code":"compile_cache","message":"Model compilation was interrupted; retry the request"})
    })?;
    let compilation = if let Some(result) = result.as_ref() {
        result.clone()
    } else if let Some(document) = compiled_hit(entries, &key)? {
        Ok(document)
    } else {
        // Only callers for this key wait here. Compilation holds neither
        // the completed-entry mutex nor the coordination-map mutex.
        compile()
    };
    if let Ok(document) = &compilation {
        let mut entries = entries.lock().map_err(|_| {
            serde_json::json!({"code":"compile_cache","message":"Compiled model cache is unavailable; restart the server"})
        })?;
        entries.retain(|(name, _)| name != &key);
        while entries.len() >= 8 {
            entries.pop_front();
        }
        entries.push_back((key, document.clone()));
    }
    *result = Some(compilation.clone());
    compilation
}

#[derive(Default)]
pub struct Cache(Mutex<VecDeque<(String, Instant, Value)>>);
impl Cache {
    pub fn put(&self, id: String, result: Value) -> bool {
        if result.to_string().len() > 4_000_000 {
            return false;
        }
        let mut c = self.0.lock().unwrap();
        c.retain(|(key, t, _)| key != &id && t.elapsed() < Duration::from_secs(900));
        while c.len() >= 8 {
            c.pop_front();
        }
        c.push_back((id, Instant::now(), result));
        true
    }
    pub fn get(&self, id: &str) -> Option<Value> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|(k, t, _)| k == id && t.elapsed() < Duration::from_secs(900))
            .map(|(_, _, v)| v.clone())
    }
}

#[cfg(test)]
mod compiled_cache_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Barrier, Condvar,
    };

    #[derive(Default)]
    struct Gate(Mutex<bool>, Condvar);
    impl Gate {
        fn wait(&self) {
            let mut open = self.0.lock().unwrap();
            while !*open {
                open = self.1.wait(open).unwrap();
            }
        }
        fn open(&self) {
            *self.0.lock().unwrap() = true;
            self.1.notify_all();
        }
    }
    fn document() -> Document {
        let model =
            crate::core::compute::model(include_str!("../assets/templates/oral.qsp")).unwrap();
        qspquest_modeling::document::CompiledDocument::compile(&model).unwrap()
    }

    #[test]
    fn different_keys_compile_without_a_shared_lock() {
        let entries = Arc::new(Mutex::new(VecDeque::new()));
        let flights = Arc::new(CompilationFlights::default());
        let gate = Arc::new(Gate::default());
        let document = document();
        let (send, receive) = std::sync::mpsc::channel();
        let threads: Vec<_> = (0..2)
            .map(|i| {
                let (entries, flights, gate, document, send) = (
                    entries.clone(),
                    flights.clone(),
                    gate.clone(),
                    document.clone(),
                    send.clone(),
                );
                std::thread::spawn(move || {
                    compiled(&entries, &flights, i.to_string(), || {
                        send.send(()).unwrap();
                        gate.wait();
                        Ok(document)
                    })
                })
            })
            .collect();
        let first = receive.recv_timeout(Duration::from_secs(2)).is_ok();
        let second = receive.recv_timeout(Duration::from_secs(2)).is_ok();
        gate.open();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        assert!(
            first && second,
            "Both compilation closures must enter before either finishes"
        );
    }

    fn identical_burst(failing: bool) {
        let entries = Arc::new(Mutex::new(VecDeque::new()));
        let flights = Arc::new(CompilationFlights::default());
        let count = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let gate = Arc::new(Gate::default());
        let document = document();
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (entries, flights, count, barrier, gate, document) = (
                    entries.clone(),
                    flights.clone(),
                    count.clone(),
                    barrier.clone(),
                    gate.clone(),
                    document.clone(),
                );
                std::thread::spawn(move || {
                    barrier.wait();
                    compiled(&entries, &flights, "same".into(), || {
                        count.fetch_add(1, Ordering::SeqCst);
                        gate.wait();
                        if failing {
                            Err(serde_json::json!({"code":"synthetic_failure"}))
                        } else {
                            Ok(document)
                        }
                    })
                })
            })
            .collect();
        let started = Instant::now();
        let mut all_waiting = false;
        while started.elapsed() < Duration::from_secs(2) {
            all_waiting = flights
                .0
                .lock()
                .unwrap()
                .get("same")
                .is_some_and(|flight| flight.strong_count() == 8);
            if all_waiting {
                break;
            }
            std::thread::yield_now();
        }
        gate.open();
        for thread in threads {
            let result = thread.join().unwrap();
            if failing {
                assert_eq!(result.err().unwrap()["code"], "synthetic_failure");
            } else {
                assert!(Arc::ptr_eq(&result.unwrap(), &document));
            }
        }
        assert!(
            all_waiting,
            "Exercise eight callers in the same live flight"
        );
        assert_eq!(count.load(Ordering::SeqCst), 1);
        if failing {
            assert!(entries.lock().unwrap().is_empty());
            compiled(&entries, &flights, "same".into(), || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(document.clone())
            })
            .unwrap();
            assert_eq!(
                count.load(Ordering::SeqCst),
                2,
                "Failure must permit a later retry"
            );
        }
    }
    #[test]
    fn simultaneous_same_key_shares_one_compilation() {
        identical_burst(false);
    }
    #[test]
    fn simultaneous_compile_failure_is_shared_and_retryable() {
        identical_burst(true);
    }

    #[test]
    fn recency_identity_and_coordination_memory_are_bounded() {
        let entries = Mutex::new(VecDeque::new());
        let flights = CompilationFlights::default();
        let document = document();
        let count = AtomicUsize::new(0);
        let get = |key: &str| {
            compiled(&entries, &flights, key.into(), || {
                count.fetch_add(1, Ordering::SeqCst);
                Ok(document.clone())
            })
            .unwrap()
        };
        get("v1:hot");
        for i in 0..100 {
            get("v1:hot");
            get(&format!("v1:other{i}"));
            assert!(entries.lock().unwrap().len() <= 8);
        }
        get("v1:hot");
        assert_eq!(count.load(Ordering::SeqCst), 101);
        get("v2:hot");
        get("v1:changed-source");
        assert_eq!(count.load(Ordering::SeqCst), 103);
        for i in 0..100 {
            assert!(compiled(&entries, &flights, format!("bad{i}"), || Err(
                serde_json::json!({"code":"synthetic_failure"})
            ))
            .is_err());
            assert!(flights.0.lock().unwrap().len() <= 1);
        }
        assert!(flights
            .0
            .lock()
            .unwrap()
            .values()
            .all(|flight| flight.strong_count() == 0));
    }
}
