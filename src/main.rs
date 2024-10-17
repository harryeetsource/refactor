use std::fs;
use syn::{File, Item};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::env;
use std::process::Command;

fn main() {
    // Get command line arguments for input file
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: refactor <input_file>");
        return;
    }

    let file_path = &args[1];
    let content = fs::read_to_string(file_path).expect("Failed to read the file");
    let input_path = Path::new(file_path);
    let output_dir = input_path.parent().expect("Failed to get parent directory");

    // Step 1: Parse the Rust source file into an AST
    let syntax_tree: File = syn::parse_file(&content).expect("Unable to parse file");

    // Step 2: Analyze the AST and group logic based on dependencies and control flow
    let mut crate_usages = Vec::new();
    let mut functions = HashMap::new();
    let mut main_function = None;
    let mut other_items = Vec::new(); // Collect other items like constants, types, etc.

    for item in &syntax_tree.items {
        match item {
            Item::Use(use_item) => {
                // Collect crate usage for analysis
                crate_usages.push(item_to_string(use_item));
            }
            Item::Fn(func) => {
                // Collect functions to group them later by name
                let func_name = func.sig.ident.to_string();
                if func_name == "main" {
                    main_function = Some(item_to_string(func));
                } else {
                    functions.insert(func_name.clone(), item_to_string(func));
                }
            }
            _ => {
                // Collect all other items (constants, types, etc.)
                other_items.push(item_to_string(item));
            }
        }
    }

    // Step 3: Refactor logic into separate files based on dependencies and function calls
    let mut mod_imports = Vec::new();
    for (func_name, func_code) in &functions {
        // Generate a valid module name from the function name
        let sanitized_name = func_name.replace('-', "_").replace(' ', "_");

        // Generate a new module file for each function with the `use crate::*;` import
        let refactored_module = format!(
            "use crate::*;\n\n{}",
            func_code
        );

        let output_path: PathBuf = output_dir.join(format!("{}_mod.rs", sanitized_name));
        let formatted_code = rustfmt_code(&refactored_module);
        fs::write(&output_path, formatted_code).expect("Failed to write the refactored file");

        // Create module import statement
        let formatted_ident = format_ident(&sanitized_name);
        mod_imports.push(formatted_ident);
    }

    // Step 4: Extract the main function and create a tmp_main.rs file with all module imports and other items
    if let Some(main_func) = main_function {
        let mut tmp_main = String::new();
        
        // Include all imports
        for import in &crate_usages {
            tmp_main.push_str(import);
            tmp_main.push('\n');
        }

        // Include all other items (constants, types, etc.)
        for item in &other_items {
            tmp_main.push_str(item);
            tmp_main.push('\n');
        }

        // Include all function module imports
        for module_import in &mod_imports {
            tmp_main.push_str(module_import);
            tmp_main.push('\n');
        }

        // Include the main function
        tmp_main.push_str(&main_func);

        let formatted_main_code = rustfmt_code(&tmp_main);

        let tmp_main_path: PathBuf = output_dir.join("tmp_main.rs");
        fs::write(tmp_main_path, formatted_main_code).expect("Failed to write the tmp_main file");
    }

    println!("Refactoring complete. Check the output files in the same directory as the input file.");
}

// Function to format Rust code using `rustfmt`
fn rustfmt_code(code: &str) -> String {
    let mut child = Command::new("rustfmt")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn rustfmt");

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(code.as_bytes()).expect("Failed to write to rustfmt stdin");
    }

    let output = child.wait_with_output().expect("Failed to read rustfmt output");
    String::from_utf8(output.stdout).expect("Failed to convert rustfmt output to string")
}

// Function to create a valid Rust module declaration from a string
fn format_ident(name: &str) -> String {
    let sanitized_name = name.replace('-', "_").replace(' ', "_");
    format!("pub mod {};", sanitized_name)
}

// Helper function to convert syn items to strings
fn item_to_string<T: quote::ToTokens>(item: &T) -> String {
    item.to_token_stream().to_string()
}
