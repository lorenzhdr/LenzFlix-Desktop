//! `package --target <triple>`: Release bauen, libmpv holen, Installer bzw. DMG erzeugen.
//!
//! Windows: NSIS (`packaging/windows/installer.nsi`), macOS: `.app` plus DMG (nur auf macOS).
//! Ausgabe in `target/dist/`.

use std::fs;
use std::path::Path;
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::process::Command;

use crate::{icons, mpv};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Opts {
    target: String,
    package: String,
    bin: String,
    skip_build: bool,
    stage_only: bool,
}

fn parse(args: &[String]) -> Res<Opts> {
    let mut o = Opts {
        target: mpv::host_target().to_string(),
        package: "lfx-app".into(),
        bin: "lenzflix".into(),
        skip_build: false,
        stage_only: false,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--target" => o.target = it.next().ok_or("--target braucht einen Wert")?.clone(),
            // Nur zum Testen: ein anderes Binary verpacken (es heißt im Paket trotzdem lenzflix).
            "--package" => o.package = it.next().ok_or("--package braucht einen Wert")?.clone(),
            "--bin" => o.bin = it.next().ok_or("--bin braucht einen Wert")?.clone(),
            "--skip-build" => o.skip_build = true,
            "--stage-only" => o.stage_only = true,
            other => return Err(format!("Unbekanntes Argument: {other}").into()),
        }
    }
    Ok(o)
}

#[cfg(target_os = "macos")]
fn run(cmd: &mut Command) -> Res<()> {
    eprintln!("> {cmd:?}");
    let st = cmd.status()?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("Befehl fehlgeschlagen ({st}): {cmd:?}").into())
    }
}

#[cfg(target_os = "macos")]
fn copy_dir_all(from: &Path, to: &Path) -> Res<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir_all(&e.path(), &dst)?;
        } else {
            fs::copy(e.path(), dst)?;
        }
    }
    Ok(())
}

/// Lizenztexte (LICENSE, Drittanbieter-Hinweise, OFL der Schriften) nach `dest` kopieren.
fn stage_licenses(root: &Path, dest: &Path) -> Res<()> {
    fs::create_dir_all(dest)?;
    fs::copy(root.join("LICENSE"), dest.join("LICENSE.txt"))?;
    fs::copy(
        root.join("THIRD-PARTY-NOTICES.md"),
        dest.join("THIRD-PARTY-NOTICES.md"),
    )?;
    let fonts = root.join("assets").join("fonts");
    let mut dirs: Vec<_> = fs::read_dir(&fonts)?.collect::<Result<_, _>>()?;
    dirs.sort_by_key(|e| e.file_name());
    for d in dirs {
        let ofl = d.path().join("OFL.txt");
        if ofl.is_file() {
            let name = d.file_name().to_string_lossy().into_owned();
            fs::copy(&ofl, dest.join(format!("OFL-{name}.txt")))?;
        }
    }
    Ok(())
}

pub fn package(args: &[String]) -> Res<()> {
    let o = parse(args)?;
    let root = mpv::workspace_root();
    let windows = o.target.contains("windows");
    let mac = o.target.contains("apple-darwin");
    if !windows && !mac {
        return Err(format!("Ziel {} wird nicht unterstützt", o.target).into());
    }

    // 1. Release bauen.
    if !o.skip_build {
        let mut cmd = Command::new("cargo");
        cmd.current_dir(&root)
            .args(["build", "--release", "-p", &o.package, "--bin", &o.bin])
            .args(["--target", &o.target]);
        eprintln!("> {cmd:?}");
        let st = cmd.status()?;
        if !st.success() {
            return Err(format!("cargo build fehlgeschlagen ({st})").into());
        }
    }
    let exe_name = if windows {
        format!("{}.exe", o.bin)
    } else {
        o.bin.clone()
    };
    let exe = root
        .join("target")
        .join(&o.target)
        .join("release")
        .join(&exe_name);
    if !exe.is_file() {
        return Err(format!("Binary fehlt: {}", exe.display()).into());
    }

    // 2. libmpv sicherstellen.
    let vendor = mpv::vendor_dir(&o.target);
    let lib = match mpv::find_lib(&vendor) {
        Some(l) => l,
        None => {
            mpv::fetch(&["--target".to_string(), o.target.clone()])?;
            mpv::find_lib(&vendor).ok_or("libmpv nach fetch-mpv nicht gefunden")?
        }
    };

    // 3. Staging.
    let stage = root.join("target").join("package").join(&o.target);
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    fs::create_dir_all(&stage)?;
    let dist = root.join("target").join("dist");
    fs::create_dir_all(&dist)?;

    if windows {
        package_windows(&root, &o, &exe, &lib, &stage, &dist)
    } else {
        package_macos(&root, &exe, &vendor, &stage, &dist)
    }
}

fn package_windows(
    root: &Path,
    o: &Opts,
    exe: &Path,
    lib: &Path,
    stage: &Path,
    dist: &Path,
) -> Res<()> {
    fs::copy(exe, stage.join("lenzflix.exe"))?;
    fs::copy(lib, stage.join("libmpv-2.dll"))?;
    fs::write(stage.join("lenzflix.ico"), icons::make_ico()?)?;
    stage_licenses(root, &stage.join("LICENSES"))?;

    let arch = if o.target.starts_with("aarch64") {
        "arm64"
    } else {
        "x64"
    };
    let outfile = dist.join(format!("LENZFLIX_{VERSION}_{arch}-setup.exe"));
    let viversion = format!("{VERSION}.0");
    let nsi = root.join("packaging").join("windows").join("installer.nsi");
    let mut cmd = Command::new("makensis");
    cmd.arg("-INPUTCHARSET")
        .arg("UTF8")
        .arg(format!("-DVERSION={VERSION}"))
        .arg(format!("-DVIVERSION={viversion}"))
        .arg(format!("-DSTAGE={}", stage.display()))
        .arg(format!("-DOUTFILE={}", outfile.display()))
        .arg(format!("-DARCH={arch}"))
        .arg(&nsi);
    eprintln!("Staging: {}", stage.display());
    if o.stage_only {
        eprintln!("--stage-only: makensis-Aufruf wäre: {cmd:?}");
        return Ok(());
    }
    match cmd.status() {
        Ok(st) if st.success() => {
            eprintln!("Installer: {}", outfile.display());
            Ok(())
        }
        Ok(st) => Err(format!("makensis fehlgeschlagen ({st})").into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(
            "makensis nicht gefunden: NSIS 3.x installieren und in den PATH legen \
             (CI: `choco install nsis`), oder mit --stage-only nur das Staging erzeugen"
                .into(),
        ),
        Err(e) => Err(e.into()),
    }
}

#[cfg(not(target_os = "macos"))]
fn package_macos(
    _root: &Path,
    _exe: &Path,
    _vendor: &Path,
    _stage: &Path,
    _dist: &Path,
) -> Res<()> {
    Err(
        "macOS-Pakete lassen sich nur auf macOS bauen (install_name_tool, codesign, hdiutil)"
            .into(),
    )
}

#[cfg(target_os = "macos")]
fn package_macos(root: &Path, exe: &Path, vendor: &Path, stage: &Path, dist: &Path) -> Res<()> {
    let app = stage.join("LENZFLIX.app");
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let frameworks = contents.join("Frameworks");
    let resources = contents.join("Resources");
    for d in [&macos, &frameworks, &resources] {
        fs::create_dir_all(d)?;
    }
    fs::copy(exe, macos.join("lenzflix"))?;
    fs::write(resources.join("lenzflix.icns"), icons::make_icns()?)?;
    stage_licenses(root, &resources.join("LICENSES"))?;
    let plist = fs::read_to_string(root.join("packaging").join("macos").join("Info.plist.in"))?
        .replace("@VERSION@", VERSION);
    fs::write(contents.join("Info.plist"), plist)?;

    // Alle Bibliotheken des Archivs nach Frameworks (libmpv und Abhängigkeiten).
    let mut dylibs = Vec::new();
    collect_dylibs(vendor, &mut dylibs)?;
    if dylibs.is_empty() {
        return Err("Keine .dylib im Vendor-Ordner".into());
    }
    let mut names = Vec::new();
    for d in &dylibs {
        let name = d
            .file_name()
            .expect("Dateiname")
            .to_string_lossy()
            .into_owned();
        fs::copy(d, frameworks.join(&name))?;
        names.push(name);
    }

    let exe_in_app = macos.join("lenzflix");
    run(Command::new("install_name_tool")
        .args(["-add_rpath", "@executable_path/../Frameworks"])
        .arg(&exe_in_app))?;
    for name in &names {
        let p = frameworks.join(name);
        run(Command::new("install_name_tool")
            .args(["-id", &format!("@rpath/{name}")])
            .arg(&p))?;
        for dep in otool_deps(&p)? {
            let base = Path::new(&dep)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if base != *name && names.contains(&base) && !dep.starts_with("@rpath/") {
                run(Command::new("install_name_tool")
                    .args(["-change", &dep, &format!("@rpath/{base}")])
                    .arg(&p))?;
            }
        }
    }
    // Zwischenlösung: lfx-mpv-sys sucht die Bibliothek nur neben der exe. Symlinks nach Frameworks.
    for n in ["libmpv.2.dylib", "libmpv.dylib"] {
        if names.iter().any(|x| x == n) {
            std::os::unix::fs::symlink(format!("../Frameworks/{n}"), macos.join(n))?;
        }
    }

    // Ad-hoc-Signatur (kein Entwicklerkonto): erst Bibliotheken, dann das Bündel.
    for name in &names {
        run(Command::new("codesign")
            .args(["--force", "--sign", "-"])
            .arg(frameworks.join(name)))?;
    }
    run(Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(&app))?;

    // DMG mit Verknüpfung auf /Applications.
    let dmg_src = stage.join("dmg");
    fs::create_dir_all(&dmg_src)?;
    copy_dir_all(&app, &dmg_src.join("LENZFLIX.app"))?;
    std::os::unix::fs::symlink("/Applications", dmg_src.join("Applications"))?;
    let out = dist.join(format!("LENZFLIX_{VERSION}_arm64.dmg"));
    let _ = fs::remove_file(&out);
    run(Command::new("hdiutil")
        .args([
            "create",
            "-volname",
            "LENZFLIX",
            "-ov",
            "-format",
            "UDZO",
            "-srcfolder",
        ])
        .arg(&dmg_src)
        .arg(&out))?;
    eprintln!("DMG: {}", out.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn collect_dylibs(dir: &Path, out: &mut Vec<PathBuf>) -> Res<()> {
    for e in fs::read_dir(dir)? {
        let p = e?.path();
        if p.is_dir() {
            collect_dylibs(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "dylib") && !p.is_symlink() {
            out.push(p);
        }
    }
    out.sort();
    Ok(())
}

/// Abhängigkeitspfade einer Mach-O-Datei laut `otool -L` (ohne die erste Zeile).
#[cfg(target_os = "macos")]
fn otool_deps(path: &Path) -> Res<Vec<String>> {
    let out = Command::new("otool").arg("-L").arg(path).output()?;
    Ok(parse_otool(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_otool(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter_map(|l| l.trim().split(" (").next().map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otool_output_is_parsed() {
        let t = "x.dylib:\n\t/opt/lib/libavcodec.61.dylib (compatibility version 61.0.0, current version 61.3.100)\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1351.0.0)\n";
        assert_eq!(
            parse_otool(t),
            vec!["/opt/lib/libavcodec.61.dylib", "/usr/lib/libSystem.B.dylib"]
        );
    }

    #[test]
    fn args_parse() {
        let o = parse(&[
            "--target".into(),
            "x86_64-pc-windows-msvc".into(),
            "--skip-build".into(),
        ])
        .unwrap();
        assert_eq!(o.target, "x86_64-pc-windows-msvc");
        assert!(o.skip_build && !o.stage_only);
        assert_eq!(o.bin, "lenzflix");
        assert!(parse(&["--foo".into()]).is_err());
    }
}
