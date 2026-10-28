//! `fuzz-shows`: random damage to valid show files. Every load must end in
//! a rejection or a valid show (and a valid show must render), never in a
//! panic. Damage: numbers out of range or huge, wrong types, missing keys,
//! huge arrays, NaN / infinity in the text. A synthetic robustness check.

use omoiiro_core::render::Renderer;
use serde_json::Value;
use std::path::PathBuf;

#[derive(clap::Args, Debug)]
pub struct FuzzShowsArgs {
    /// Valid show files to start from.
    #[arg(required = true)]
    pub files: Vec<PathBuf>,
    /// Damaged copies per file.
    #[arg(long, default_value_t = 1000)]
    pub iterations: u32,
    /// Random seed (the same seed gives the same damage).
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
}

/// xorshift64*: enough for picking damage, no extra crate.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// JSON Pointers of every node.
fn paths(v: &Value, here: String, out: &mut Vec<String>) {
    out.push(here.clone());
    match v {
        Value::Object(m) => m.iter().for_each(|(k, x)| paths(x, format!("{here}/{}", k.replace('~', "~0").replace('/', "~1")), out)),
        Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| paths(x, format!("{here}/{i}"), out)),
        _ => {}
    }
}

const NUMBERS: [f64; 9] = [-1.0, 0.0, 1e9, -1e9, 1e308, -1e308, 0.5, 9_007_199_254_740_993.0, 65_536.0];

/// One damage at a random node; returns its description.
fn damage(doc: &mut Value, rng: &mut Rng) -> String {
    let mut all = Vec::new();
    paths(doc, String::new(), &mut all);
    let p = all[rng.below(all.len())].clone();
    let (parent, key) = match p.rfind('/') {
        Some(i) => (p[..i].to_string(), p[i + 1..].to_string()),
        None => (String::new(), String::new()),
    };
    let kind = rng.below(7);
    let node = doc.pointer_mut(&p).unwrap();
    match kind {
        0 => {
            *node = Value::from(NUMBERS[rng.below(NUMBERS.len())]);
            format!("{p}: number {node}")
        }
        1 => {
            let to = match node {
                Value::String(_) => Value::from(rng.below(100) as i64),
                Value::Number(_) => Value::from("12"),
                Value::Bool(_) => Value::Array(vec![]),
                Value::Array(_) => Value::from(true),
                Value::Object(_) => Value::from("{}"),
                Value::Null => Value::from(0),
            };
            *node = to;
            format!("{p}: type changed")
        }
        2 => {
            // Remove the key (or array item) from its parent.
            match doc.pointer_mut(&parent) {
                Some(Value::Object(m)) => {
                    m.remove(&key.replace("~1", "/").replace("~0", "~"));
                }
                Some(Value::Array(a)) => {
                    if let Ok(i) = key.parse::<usize>() {
                        if i < a.len() {
                            a.remove(i);
                        }
                    }
                }
                _ => {}
            }
            format!("{p}: removed")
        }
        3 => {
            // A huge array: the node repeated 20 000 times.
            let item = node.clone();
            *node = Value::Array(vec![item; 20_000]);
            format!("{p}: 20000-item array")
        }
        4 => {
            *node = Value::Null;
            format!("{p}: null")
        }
        5 => {
            *node = Value::from("\u{1F525}".repeat(rng.below(200) + 1));
            format!("{p}: long text")
        }
        _ => {
            // Marked for NaN / Infinity in the text (not representable as a JSON value).
            *node = Value::from("__NAN__");
            format!("{p}: NaN in the text")
        }
    }
}

/// None = fine, Some(reason) = a panic.
fn try_one(text: &str) -> (Option<String>, bool) {
    let outcome = std::panic::catch_unwind(|| {
        let Ok(show) = omoiiro_store::load_str(text) else { return false };
        let mut r = Renderer::new(&show).expect("a loaded show renders");
        let end = omoiiro_runtime::preview::show_length_us(&show).unwrap_or(0).max(1);
        for t in [0, end / 2, end - 1] {
            let _ = r.render(t, None);
        }
        true
    });
    match outcome {
        Ok(accepted) => (None, accepted),
        Err(p) => (Some(p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())), false),
    }
}

pub fn run(args: FuzzShowsArgs) -> Result<(), Box<dyn std::error::Error>> {
    // The panics are counted and listed here; the default hook would print each one.
    std::panic::set_hook(Box::new(|_| {}));
    let mut rng = Rng(args.seed.max(1));
    let (mut total, mut accepted, mut panics) = (0u32, 0u32, Vec::new());
    for f in &args.files {
        let base: Value = serde_json::from_str(&std::fs::read_to_string(f)?)?;
        for i in 0..args.iterations {
            let mut doc = base.clone();
            let n = 1 + rng.below(3);
            let what: Vec<String> = (0..n).map(|_| damage(&mut doc, &mut rng)).collect();
            let mut text = serde_json::to_string_pretty(&doc)?;
            for token in ["NaN", "Infinity", "-Infinity"] {
                if text.contains("\"__NAN__\"") {
                    text = text.replacen("\"__NAN__\"", token, 1);
                }
            }
            let (panic, ok) = try_one(&text);
            total += 1;
            accepted += ok as u32;
            if let Some(p) = panic {
                panics.push(format!("{} #{i}: {p} after [{}]", f.display(), what.join("; ")));
            }
        }
    }
    let _ = std::panic::take_hook();
    println!("{total} damaged shows: {accepted} accepted (and rendered), {} rejected, {} panics", total - accepted - panics.len() as u32, panics.len());
    for p in panics.iter().take(20) {
        println!("PANIC {p}");
    }
    if panics.is_empty() {
        Ok(())
    } else {
        Err(format!("{} panics", panics.len()).into())
    }
}