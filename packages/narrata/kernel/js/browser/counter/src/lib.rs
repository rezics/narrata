//! The test counter domain over a [`CacheBackend`], exported to JavaScript so the browser tests
//! can drive the IndexedDB adapter with the real cache and engine.
//!
//! Every operation may fail because the cache lacks something; the page's `StorageHost` then
//! takes the request, loads it and calls the operation again, and returns its result only after
//! the batches are persisted.

#![forbid(unsafe_code)]

use narrata_history::{
    DomainKinds, History, HistoryError, RefKey, RefName, Session, testing::Counter,
};
use narrata_storage_host::{CacheBackend, FlushReply, HostError, Loaded};
use wasm_bindgen::prelude::*;

const COUNTER: Counter = Counter::new(1_000_000);

fn player() -> Result<RefName, JsError> {
    Ok(RefName::new("player")?)
}

fn state(value: u64) -> Result<u32, JsError> {
    u32::try_from(value).map_err(|_| JsError::new("counter state does not fit in 32 bits"))
}

type Counters = History<CacheBackend, DomainKinds<Counter>>;

/// One tab's counter session; the host keeps it in step with IndexedDB.
#[wasm_bindgen]
#[derive(Default)]
pub struct CounterSaves {
    cache: CacheBackend,
    session: Option<(Counters, Session<Counter>)>,
    clock: u64,
}

#[wasm_bindgen]
impl CounterSaves {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }

    /// The encoded load request for what failed calls lacked, if anything.
    #[wasm_bindgen(js_name = takeRequest)]
    pub fn take_request(&self) -> Option<Vec<u8>> {
        self.cache.take_request().map(|request| request.encode())
    }

    /// Adds an encoded load answer; `false` when another tab moved the store under unconfirmed
    /// batches, after which the session must be opened again.
    pub fn load(&mut self, loaded: &[u8]) -> Result<bool, JsError> {
        let loaded = Loaded::decode(loaded)?;
        let result = self.cache.load(&loaded);
        self.settle(result)
    }

    /// The encoded flush of the batches awaiting persistence, if any.
    pub fn unconfirmed(&self) -> Option<Vec<u8>> {
        self.cache.unconfirmed().map(|flush| flush.encode())
    }

    /// Applies an encoded flush reply; `false` when another tab wrote first, after which the
    /// session must be opened again.
    pub fn confirm(&mut self, reply: &[u8]) -> Result<bool, JsError> {
        let reply = FlushReply::decode(reply)?;
        let result = self.cache.confirm(&reply);
        self.settle(result)
    }

    fn settle(&mut self, result: Result<(), HostError>) -> Result<bool, JsError> {
        match result {
            Ok(()) => Ok(true),
            Err(HostError::Superseded(_)) => {
                self.session = None;
                Ok(false)
            }
            Err(error) => Err(error.into()),
        }
    }

    fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    /// Drops the session and what the cache holds, which may predate another tab's writes, so
    /// that the next `open` reads the store again. Batches awaiting persistence stay.
    pub fn reload(&mut self) {
        self.session = None;
        self.cache.forget();
    }

    /// Opens the session `player`, creating it at 0 in an empty store, and returns its state.
    /// The host may call it several times while it loads what the cache lacks.
    pub fn open(&mut self) -> Result<u32, JsError> {
        self.session = None;
        let observed = self.tick();
        let mut history = History::open(self.cache.share(), DomainKinds::new())?;
        let (session, loaded) = match Session::open(&history, COUNTER, player()?) {
            Err(HistoryError::MissingRef(_)) => {
                Session::create(&mut history, COUNTER, player()?, &0, observed)?
            }
            other => other?,
        };
        self.session = Some((history, session));
        state(loaded.state)
    }

    /// Adds `increment` to the counter and returns the new state.
    pub fn advance(&mut self, increment: u32) -> Result<u32, JsError> {
        let observed = self.tick();
        let (history, session) = self.opened()?;
        let head = session.head();
        let advanced = session.advance(
            history,
            head,
            &u64::from(increment),
            |counter, state, increment| counter.step(state, increment),
            observed,
        )?;
        state(advanced.state)
    }

    /// Points the save slot `quick` at the session's head.
    pub fn save(&mut self) -> Result<(), JsError> {
        let observed = self.tick();
        let (history, session) = self.opened()?;
        let slot = RefKey::save(player()?, RefName::new("quick")?);
        let expected = history.read_ref(&slot)?.map(|value| value.revision);
        session.save(history, slot, expected, observed)?;
        Ok(())
    }

    fn opened(&mut self) -> Result<(&mut Counters, &mut Session<Counter>), JsError> {
        let (history, session) = self
            .session
            .as_mut()
            .ok_or_else(|| JsError::new("open the session first"))?;
        Ok((history, session))
    }
}
