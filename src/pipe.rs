use std::io::{self, IsTerminal, Write};
use std::path::Path;

pub fn read_input(input_arg: &str) -> io::Result<String> {
    if input_arg == "-" {
        let stdin = io::stdin();
        if stdin.is_terminal() {
            eprintln!("reading minified JS from stdin... (Ctrl+D when done)");
        }
        io::read_to_string(stdin)
    } else {
        std::fs::read_to_string(input_arg)
    }
}

pub fn write_output(output_arg: Option<&Path>, contents: &str) -> io::Result<()> {
    match output_arg {
        None => {
            io::stdout().write_all(contents.as_bytes())?;
            Ok(())
        }
        Some(path) => std::fs::write(path, contents),
    }
}

#[cfg(test)]
mod tests {
    // DSL not applied: pipe tests exercise OS I/O (tempfiles, stdin path) and are
    // already 3–5 lines each. A DSL wrapper would add boilerplate without clarity.
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn read_input_from_file_round_trips() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "hello").unwrap();
        let path = f.path().to_str().unwrap().to_string();
        assert_eq!(read_input(&path).unwrap(), "hello");
    }

    #[test]
    fn read_input_missing_file_errors() {
        let result = read_input("/nonexistent/path/that/does/not/exist.js");
        assert!(result.is_err());
    }

    #[test]
    fn write_output_to_file() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_owned();
        write_output(Some(&path), "x").unwrap();
        let got = std::fs::read_to_string(&path).unwrap();
        assert_eq!(got, "x");
    }

    #[test]
    fn write_output_to_file_overwrites() {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "old content that should be gone").unwrap();
        f.flush().unwrap();
        let path = f.path().to_owned();
        write_output(Some(&path), "new").unwrap();
        let got = std::fs::read_to_string(&path).unwrap();
        assert_eq!(got, "new");
    }

    #[test]
    fn read_input_non_utf8_errors() {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&[0xFF, 0xFE, 0x00]).unwrap();
        f.flush().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        assert!(read_input(&path).is_err());
    }
}
