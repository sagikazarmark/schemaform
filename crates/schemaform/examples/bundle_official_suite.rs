use std::{
    env,
    error::Error,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::{Value, json};

const SUITE_REVISION: &str = "c0b038ad7244712cf73650f44e90d0bc5704e8c7";
const OPTIONAL_FILES: [&str; 4] = [
    "bignum.json",
    "ecmascript-regex.json",
    "float-overflow.json",
    "non-bmp-regex.json",
];

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os().skip(1);
    let suite_root = PathBuf::from(required_argument(&mut arguments, "suite checkout")?);
    let output = PathBuf::from(required_argument(&mut arguments, "output file")?);
    if arguments.next().is_some() {
        return Err("expected exactly two arguments".into());
    }
    verify_revision(&suite_root)?;

    let tests_root = suite_root.join("tests/draft2020-12");
    let mandatory = immediate_json_files(&tests_root)?
        .into_iter()
        .map(|path| suite_file(&path))
        .collect::<Result<Vec<_>, _>>()?;
    let optional = OPTIONAL_FILES
        .iter()
        .map(|file| suite_file(&tests_root.join("optional").join(file)))
        .collect::<Result<Vec<_>, _>>()?;

    let remotes_root = suite_root.join("remotes/draft2020-12");
    let mut remote_paths = Vec::new();
    collect_json_files(&remotes_root, &mut remote_paths)?;
    remote_paths.sort();
    let remotes = remote_paths
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&remotes_root)
                .expect("collected resources should remain under the remotes root")
                .to_string_lossy()
                .replace('\\', "/");
            Ok(json!({
                "uri": format!("http://localhost:1234/draft2020-12/{relative}"),
                "schema": read_json(&path)?,
            }))
        })
        .collect::<Result<Vec<Value>, Box<dyn Error>>>()?;

    let bundle = json!({
        "source": "https://github.com/json-schema-org/JSON-Schema-Test-Suite",
        "revision": SUITE_REVISION,
        "mandatory": mandatory,
        "optional": optional,
        "remotes": remotes,
    });
    let mut encoded = serde_json::to_vec(&bundle)?;
    encoded.push(b'\n');
    fs::write(&output, encoded)
        .map_err(|error| io_error(&output, "write generated bundle", error))?;

    Ok(())
}

fn required_argument(
    arguments: &mut impl Iterator<Item = OsString>,
    name: &str,
) -> Result<OsString, Box<dyn Error>> {
    arguments
        .next()
        .ok_or_else(|| format!("missing {name} argument").into())
}

fn verify_revision(suite_root: &Path) -> Result<(), Box<dyn Error>> {
    let revision_output = Command::new("git")
        .arg("-C")
        .arg(suite_root)
        .args(["rev-parse", "HEAD"])
        .output()?;
    let revision = String::from_utf8(revision_output.stdout)?;
    if !revision_output.status.success() || revision.trim() != SUITE_REVISION {
        return Err(format!(
            "suite checkout must be at {SUITE_REVISION}, found {}",
            revision.trim()
        )
        .into());
    }

    let status_output = Command::new("git")
        .arg("-C")
        .arg(suite_root)
        .args([
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--",
            "tests/draft2020-12",
            "remotes/draft2020-12",
        ])
        .output()?;
    if !status_output.status.success() {
        return Err("failed to inspect the suite checkout worktree".into());
    }
    let changes = String::from_utf8(status_output.stdout)?;
    if !changes.is_empty() {
        return Err(format!(
            "suite inputs must match {SUITE_REVISION}; relevant worktree changes were found:\n{changes}"
        )
        .into());
    }
    Ok(())
}

fn immediate_json_files(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = fs::read_dir(directory)
        .map_err(|error| io_error(directory, "read suite directory", error))?
        .filter_map(|entry| match entry {
            Ok(entry)
                if entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "json") =>
            {
                Some(Ok(entry.path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    files.sort();
    Ok(files)
}

fn collect_json_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)
        .map_err(|error| io_error(directory, "read remote-resource directory", error))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_json_files(&entry.path(), files)?;
        } else if file_type.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn suite_file(path: &Path) -> Result<Value, Box<dyn Error>> {
    let file = path
        .file_name()
        .ok_or_else(|| format!("suite path has no filename: {}", path.display()))?
        .to_string_lossy();
    Ok(json!({
        "file": file,
        "groups": read_json(path)?,
    }))
}

fn read_json(path: &Path) -> Result<Value, Box<dyn Error>> {
    let source = fs::read(path).map_err(|error| io_error(path, "read JSON", error))?;
    serde_json::from_slice(&source).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("failed to parse {}: {error}", path.display()),
        )
        .into()
    })
}

fn io_error(path: &Path, operation: &str, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("failed to {operation} {}: {error}", path.display()),
    )
}
