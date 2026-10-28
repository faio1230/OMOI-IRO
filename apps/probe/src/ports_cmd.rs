//! `ports` printing. Listing is `omoiiro_midi::ports::list`.

use omoiiro_midi::ports::{self, PortList};

pub fn print(list: &PortList, json: bool) {
    if json {
        let v = serde_json::json!({
            "backend": ports::backend_name(),
            "inputs": list.inputs,
            "outputs": list.outputs,
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return;
    }
    println!("backend: {}", ports::backend_name());
    println!("inputs ({}):", list.inputs.len());
    for (i, n) in list.inputs.iter().enumerate() {
        println!("  #{i}  {n}");
    }
    println!("outputs ({}):", list.outputs.len());
    for (i, n) in list.outputs.iter().enumerate() {
        println!("  #{i}  {n}");
    }
}
