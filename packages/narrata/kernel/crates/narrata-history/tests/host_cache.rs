//! Sessions over a cache whose host stands in for IndexedDB (ADR 0017): every operation loads
//! what it lacks and runs again, and its batches reach the host before its result is used.

#![allow(clippy::panic, clippy::unwrap_used)]

use std::fmt::Debug;

use narrata_history::{
    AdvanceError, DomainKinds, History, HistoryError, RefKey, RefName, Session,
    testing::{Counter, Overflow},
};
use narrata_storage_host::{
    CacheBackend, HostError, StoreId,
    testing::{HostReads, MemoryHost, RunError, load, run},
};

type Counters = History<CacheBackend, DomainKinds<Counter>>;

const COUNTER: Counter = Counter::new(1_000_000);

fn name(value: &str) -> RefName {
    RefName::new(value).unwrap()
}

fn step(counter: &Counter, state: &u64, input: &u64) -> Result<u64, Overflow> {
    counter.step(state, input)
}

/// One browser tab: a cache, the engine over it, and the session `player`.
struct Tab {
    cache: CacheBackend,
    history: Counters,
    session: Session<Counter>,
    state: u64,
}

impl Tab {
    /// Opens the session with a new cache, creating it at 0 in an empty store.
    fn open(host: &MemoryHost) -> Self {
        let cache = CacheBackend::new();
        let mut history = run(&cache, host, || {
            History::open(cache.share(), DomainKinds::new())
        })
        .unwrap();
        let (session, loaded) = run(&cache, host, || {
            match Session::open(&history, COUNTER, name("player")) {
                Err(HistoryError::MissingRef(_)) => {
                    Session::create(&mut history, COUNTER, name("player"), &0, 0)
                }
                other => other,
            }
        })
        .unwrap();
        Self {
            cache,
            history,
            session,
            state: loaded.state,
        }
    }

    fn advance(
        &mut self,
        host: &MemoryHost,
        increment: u64,
    ) -> Result<u64, RunError<AdvanceError<HistoryError, Overflow>>> {
        let advanced = run(&self.cache, host, || {
            let head = self.session.head();
            self.session
                .advance(&mut self.history, head, &increment, step, 1)
        })?;
        self.state = advanced.state;
        Ok(advanced.state)
    }

    /// Advances without letting the host persist anything, as when the page dies before the
    /// transaction completes.
    fn advance_unconfirmed(&mut self, host: &MemoryHost, increment: u64) {
        for _ in 0..16 {
            let head = self.session.head();
            match self
                .session
                .advance(&mut self.history, head, &increment, step, 1)
            {
                Ok(_) => return,
                Err(error) => assert!(load(&self.cache, host).unwrap(), "{error:?}"),
            }
        }
        panic!("the advance kept missing");
    }

    fn save(&mut self, host: &MemoryHost) -> Result<(), RunError<HistoryError>> {
        let slot = RefKey::save(name("player"), name("quick"));
        run(&self.cache, host, || {
            let expected = self.history.read_ref(&slot)?.map(|value| value.revision);
            self.session
                .save(&mut self.history, slot.clone(), expected, 1)
        })?;
        Ok(())
    }
}

fn superseded<T: Debug, E: Debug>(result: Result<T, RunError<E>>) {
    assert!(
        matches!(result, Err(RunError::Host(HostError::Superseded(_)))),
        "{result:?}"
    );
}

#[test]
fn reopening_finds_the_last_confirmed_step() {
    let host = MemoryHost::new(StoreId::from_bytes([1; 16]));
    let mut tab = Tab::open(&host);
    for _ in 0..3 {
        tab.advance(&host, 2).unwrap();
    }
    tab.advance_unconfirmed(&host, 100);
    assert!(tab.cache.unconfirmed().is_some());
    drop(tab);

    let mut reopened = Tab::open(&host);
    assert_eq!(
        reopened.state, 6,
        "the unconfirmed step was lost with its page"
    );
    assert_eq!(reopened.advance(&host, 1).unwrap(), 7);
    assert_eq!(Tab::open(&host).state, 7);
}

fn reads_of_one_step(host: &MemoryHost, tab: &mut Tab, depth: u64) -> HostReads {
    while tab.state < depth {
        tab.advance(host, 1).unwrap();
    }
    host.take_reads();
    tab.advance(host, 1).unwrap();
    host.take_reads()
}

fn reads_of_reopening(host: &MemoryHost) -> HostReads {
    host.take_reads();
    Tab::open(host);
    host.take_reads()
}

#[test]
fn host_reads_do_not_grow_with_depth() {
    let host = MemoryHost::new(StoreId::from_bytes([2; 16]));
    // The tab that creates a store knows all of it and never loads; measure one that opened it.
    Tab::open(&host);
    let mut tab = Tab::open(&host);
    let shallow = reads_of_one_step(&host, &mut tab, 8);
    let shallow_reopen = reads_of_reopening(&host);
    let deep = reads_of_one_step(&host, &mut tab, 128);
    let deep_reopen = reads_of_reopening(&host);
    assert_eq!(shallow, deep);
    assert_eq!(shallow_reopen, deep_reopen);
    // A step asks for its transition key, then learns that its new state and commit are absent;
    // the input object is the same at every step.
    assert_eq!(
        deep,
        HostReads {
            loads: 2,
            keys: 1,
            objects: 2,
            ..HostReads::default()
        }
    );
    // Reopening reads the layout marker, the cursor, its commit and state, and the session's
    // one branch, one after another.
    assert_eq!(
        deep_reopen,
        HostReads {
            loads: 5,
            keys: 2,
            objects: 2,
            key_entries: 1,
            object_entries: 0,
        }
    );
}

#[test]
fn a_second_tab_never_overwrites_the_first() {
    let host = MemoryHost::new(StoreId::from_bytes([3; 16]));
    let mut first = Tab::open(&host);
    let mut second = Tab::open(&host);
    second.save(&host).unwrap();

    // The first tab created the store and knows all of it, so its step loads nothing and only
    // the host's revision check at the flush notices the second tab's save.
    superseded(first.advance(&host, 1));
    assert!(first.cache.store().is_none());
    let mut first = Tab::open(&host);
    assert_eq!(first.advance(&host, 1).unwrap(), 1);

    // The second tab's step needs its transition key; that load finds the store moved, and the
    // cursor read again is not where the stale session stands.
    assert!(matches!(
        second.advance(&host, 10),
        Err(RunError::Failed(AdvanceError::History(
            HistoryError::HeadMoved { .. }
        )))
    ));
    let mut second = Tab::open(&host);
    assert_eq!(second.state, 1);
    assert_eq!(second.advance(&host, 10).unwrap(), 11);
    assert_eq!(Tab::open(&host).state, 11);
}
