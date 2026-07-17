//! Common helpers for microbenchmarks.

use std::fs;

use tempfile::TempDir;
use url::Url;

/// Default number of iterations to run.
const DEFAULT_ITERATIONS: usize = 10000;

/// Parse the number of iterations to run from the CLI.
pub fn iterations() -> usize {
    let args: Vec<String> = std::env::args().collect();
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if (arg == "-n")
            && let Some(val) = iter.next()
            && let Ok(n) = val.parse::<usize>()
        {
            return n;
        }
    }
    DEFAULT_ITERATIONS
}

/// Workspace setup for benchmarks that test large import chains and documents.
pub struct LargeWorkspace {
    /// Temp directory holding the WDL files.
    #[expect(unused, reason = "to keep it alive")]
    pub temp: TempDir,
    /// Dependencies in the import chain.
    pub dependencies: Vec<Url>,
    /// Main document URL.
    pub main: Url,
}

impl LargeWorkspace {
    /// The number of documents in the import chain.
    pub const IMPORTS_DEPTH: usize = 15;
    /// The number of tasks to put in the main document.
    pub const MAIN_DOC_TASKS: usize = 50;

    /// Set up a new [`LargeWorkspace`].
    pub fn setup() -> LargeWorkspace {
        let temp = TempDir::new().unwrap();
        let mut urls = Vec::new();

        for i in 0..Self::IMPORTS_DEPTH {
            let content = format!(
                r#"version 1.3

{import}

task say_hello_{i} {{
    input {{
        String s
    }}

    command <<<
        echo "Hello, ~{{s}}!"
    >>>

    output {{
        String out = read_string(stdout())
    }}
}}
                "#,
                import = if i > 0 {
                    format!("import \"lib_{}.wdl\"", i - 1)
                } else {
                    String::new()
                }
            );
            let path = temp.path().join(format!("lib_{i}.wdl"));
            fs::write(&path, content).unwrap();
            urls.push(Url::from_file_path(path).unwrap());
        }

        let mut main = format!(
            r#"version 1.3

import "lib_{import_depth}.wdl"

"#,
            import_depth = Self::IMPORTS_DEPTH - 1
        );

        for i in 0..Self::MAIN_DOC_TASKS {
            main.push_str(&format!(
                r#"task say_hello_{i} {{
    input {{
        String name
    }}

    command <<<
        echo "Hello, ~{{name}}!"
    >>>

    output {{
        String out = read_string(stdout())
    }}
}}

"#,
            ));
        }

        main.push_str(
            r"workflow main {
    input {
        String name
    }

",
        );
        for i in 0..Self::MAIN_DOC_TASKS {
            main.push_str(&format!("    call say_hello_{i} {{ name }}\n"));
        }
        main.push_str(
            r"    output {
        String out = say_hello_0.out
    }
}
",
        );

        let main_path = temp.path().join("main.wdl");
        fs::write(&main_path, main).unwrap();
        let main_url = Url::from_file_path(main_path).unwrap();

        LargeWorkspace {
            temp,
            dependencies: urls,
            main: main_url,
        }
    }
}
