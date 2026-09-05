use std::sync::Arc;

use narrata_nodes::{
    Bundle, CheckedProduct, NodeRegistry, Session, SessionView, compile, parse_json,
};
use serde_json::{Value, json};

pub type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

pub fn source() -> Value {
    json!({
        "format_version": 1,
        "product": {"id":"test", "title":"组合测试", "entry":{"package":"main","graph":"journey"},
            "shared":{"visits":0,"unlocked":false},
            "bindings":[{"from":{"package":"main","graph":"journey","port":"visit"},"to":{"package":"camp","graph":"visit"}}]},
        "packages": {
            "main": {"id":"example.main","version":"1","exports":["journey"],
                "content":{"start":{"title":"入口","paragraphs":["已访问 {{shared.visits}} 次"]}},
                "graphs":{"journey":{"title":"主线","entry":"start","outcomes":["done"],"shared":{"visits":"int","unlocked":"bool"},
                    "imports":{"visit":{"parameters":{"name":"text"},"outcomes":["done"]}},
                    "nodes":{
                        "start":{"type_id":"narrata.decision","data":{"content":"start","choices":[
                            {"id":"visit","label":"访问营地","target":"call"},
                            {"id":"locked","label":"上锁的路线","enabled_if":{"kind":"read","scope":"shared","name":"unlocked"},"disabled_reason":"尚未解锁","target":"end"},
                            {"id":"hidden","label":"秘密路线","visible_if":{"kind":"read","scope":"shared","name":"unlocked"},"target":"end"},
                            {"id":"finish","label":"结束","target":"end"}]}},
                        "call":{"type_id":"narrata.call","data":{"target":{"kind":"import","port":"visit"},"arguments":{"name":{"kind":"literal","value":"阿岚"}},"on_return":{"done":"start"}}},
                        "end":{"type_id":"narrata.return","data":{"outcome":"done"}}
                    }}}},
            "camp": {"id":"example.camp","version":"1","exports":["visit"],
                "content":{"camp":{"title":"营地","paragraphs":["{{parameter.name}}，本次局部计数 {{local.count}}，累计 {{shared.visits}}。"]}},
                "graphs":{"visit":{"title":"营地互动","parameters":{"name":"text"},"locals":{"count":0},"shared":{"visits":"int"},"entry":"increment","outcomes":["done"],
                    "nodes":{
                        "increment":{"type_id":"narrata.mutate","data":{"next":"page","assignments":[
                            {"target":{"scope":"local","name":"count"},"value":{"kind":"binary","op":"add","left":{"kind":"read","scope":"local","name":"count"},"right":{"kind":"literal","value":1}}},
                            {"target":{"scope":"shared","name":"visits"},"value":{"kind":"binary","op":"add","left":{"kind":"read","scope":"shared","name":"visits"},"right":{"kind":"literal","value":1}}}]}},
                        "page":{"type_id":"narrata.content","data":{"content":"camp","label":"返回主线","next":"end"}},
                        "end":{"type_id":"narrata.return","data":{"outcome":"done"}}
                    }}}}
        }
    })
}

pub fn product(raw: Value) -> TestResult<Arc<CheckedProduct>> {
    let bundle: Bundle = parse_json(&raw.to_string())?;
    Ok(Arc::new(
        compile(bundle, &NodeRegistry::gamebook())?.product,
    ))
}

pub fn click(session: &mut Session, action: &str) -> TestResult<SessionView> {
    let head = session.cursor()?.to_owned();
    Ok(session.select(&head, action)?)
}

pub fn compile_error(raw: Value) -> TestResult<String> {
    let bundle: Bundle = parse_json(&raw.to_string())?;
    Ok(compile(bundle, &NodeRegistry::gamebook())
        .err()
        .ok_or("expected compile failure")?
        .code)
}
