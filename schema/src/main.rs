use pesde::manifest::Manifest;
use schemars::schema_for;
use std::fs;

fn main() {
	let schema = schema_for!(Manifest);
	let content = serde_json::to_string_pretty(&schema).unwrap();

	fs::create_dir_all("generated").unwrap();

	fs::write("generated/pesde.json", content).unwrap();
}
