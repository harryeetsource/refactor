use std::fs;
use syn::{File, Item, visit::Visit, UseTree};
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use std::env;
use std::process::Command;
use std::collections::HashSet;
use syn::visit::visit_item_fn;
use syn::{ExprPath, ItemFn};

struct CrateUsageVisitor<'a> {
    imported_functions: &'a HashMap<String, String>,
    used_crates: HashSet<String>,
}

impl<'a> Visit<'_> for CrateUsageVisitor<'a> {
    fn visit_expr_path(&mut self, node: &ExprPath) {
        if let Some(segment) = node.path.segments.first() {
            let func_name = segment.ident.to_string();
            if let Some(crate_name) = self.imported_functions.get(&func_name) {
                self.used_crates.insert(crate_name.clone());
            }
        }
        syn::visit::visit_expr_path(self, node);
    }
}

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
    let mut imported_functions = HashMap::new();
    let mut functions = HashMap::new();
    let mut main_function = None;
    let mut other_items = Vec::new(); // Collect other items like constants, types, etc.

    for item in &syntax_tree.items {
        match item {
            Item::Use(use_item) => {
                // Collect crate usage and the functions imported from each crate
                if let UseTree::Path(use_path) = &use_item.tree {
                    let crate_name = use_path.ident.to_string();
                    imported_functions.insert(crate_name.clone(), item_to_string(use_item));
                }
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

    // Step 3: Group functions into modules based on subcrate dependencies
    let mut grouped_functions: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut group_imports: HashMap<String, HashSet<String>> = HashMap::new();
    let mut group_counter = 1;

    for (func_name, func_code) in &functions {
        let mut visitor = CrateUsageVisitor {
            imported_functions: &imported_functions,
            used_crates: HashSet::new(),
        };
        let func_ast: ItemFn = syn::parse_str(func_code).expect("Unable to parse function AST");
        visit_item_fn(&mut visitor, &func_ast);

        let used_crates = visitor.used_crates;
        let group_name = if !used_crates.is_empty() {
            format!("group_{}", group_counter)
        } else {
            "general".to_string()
        };

        if used_crates.is_empty() {
            grouped_functions.entry(group_name.clone()).or_default().push((func_name.clone(), func_code.clone()));
            group_imports.entry(group_name.clone()).or_default().extend(used_crates);
        } else {
            if !grouped_functions.contains_key(&group_name) {
                group_counter += 1;
            }
            grouped_functions.entry(group_name.clone()).or_default().push((func_name.clone(), func_code.clone()));
            group_imports.entry(group_name.clone()).or_default().extend(used_crates);
        }
    }

    let mut mod_declarations = Vec::new();
    let mut use_statements = Vec::new();

    // Step 4: Refactor logic into separate files based on grouped functions
    for (group_name, funcs) in &grouped_functions {
        if group_name == "general" && funcs.len() == functions.len() {
            // Skip creating a general_mod if all functions are grouped as general
            continue;
        }
        
        // Sanitize the module name to remove invalid characters
        let sanitized_group_name = sanitize_filename(group_name);
        let module_name = format!("{}_mod", sanitized_group_name);
        let mut module_code = String::new();

        // Add `use crate::*;` to import everything from the main file
        module_code.push_str("use crate::*;\n\n");

        // Include relevant imports for this module
        if let Some(imports) = group_imports.get(group_name) {
            for import in imports {
                if let Some(import_code) = imported_functions.get(import) {
                    module_code.push_str(import_code);
                    module_code.push_str("\n");
                }
            }
        }
        module_code.push_str("\n");

        // Add the functions to the module
        for (_func_name, func_code) in funcs {
            module_code.push_str(func_code);
            module_code.push_str("\n\n");
        }

        let output_path: PathBuf = output_dir.join(format!("{}.rs", module_name));
        let formatted_code = rustfmt_code(&module_code);
        fs::write(&output_path, formatted_code).unwrap_or_else(|e| panic!("Failed to write the refactored file: {:?} with error: {}", output_path, e));

        // Create module declaration and use statement
        mod_declarations.push(format!("pub mod {};", module_name));
        use_statements.push(format!("pub use {}::*;", module_name));
    }

    // Step 5: Extract the main function and create a tmp_main.rs file with all module imports and other items
    if let Some(main_func) = main_function {
        let mut tmp_main = String::new();
        
        // Include all imports not associated with any function group
        for import in imported_functions.values() {
            tmp_main.push_str(import);
            tmp_main.push_str("\n\n");
        }

        // Include all other items (constants, types, etc.)
        for item in &other_items {
            tmp_main.push_str(item);
            tmp_main.push_str("\n\n");
        }

        // Include all function module declarations
        for mod_decl in &mod_declarations {
            tmp_main.push_str(mod_decl);
            tmp_main.push_str("\n\n");
        }
        
        // Include all function public use imports
        for use_statement in &use_statements {
            tmp_main.push_str(use_statement);
            tmp_main.push_str("\n\n");
        }

        // Include the main function
        tmp_main.push_str(&main_func);
        tmp_main.push_str("\n\n");

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

// Helper function to convert syn items to strings
fn item_to_string<T: quote::ToTokens>(item: &T) -> String {
    item.to_token_stream().to_string()
}

// Function to sanitize a filename by removing invalid characters
fn sanitize_filename(filename: &str) -> String {
    filename.chars().filter(|c| c.is_alphanumeric() || *c == '_').collect()
}
