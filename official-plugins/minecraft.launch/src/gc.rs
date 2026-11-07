use super::{prepare::Java, services::Child};
use hydrogen_plugin_services::contracts::{ApiError, Result};
use std::{
    path::Path,
    time::{Duration, Instant},
};

const G1: &[&str] = &["-XX:+UseG1GC"];
const TUNED_G1: &[&str] = &[
    "-XX:+UseG1GC",
    "-XX:+ParallelRefProcEnabled",
    "-XX:MaxGCPauseMillis=100",
    "-XX:G1ReservePercent=20",
    "-XX:InitiatingHeapOccupancyPercent=15",
];

pub fn arguments(
    mode: &str,
    java: &Java,
    existing: &[String],
    mut supports: impl FnMut(&[&str]) -> Result<bool>,
) -> Result<Vec<String>> {
    if !["preferZgc", "preferGenerationalZgc", "g1", "tunedG1"].contains(&mode) {
        return Err(ApiError::new("INVALID_SETTINGS", "内存设置无效。"));
    }
    // Preserve explicit collector choices in custom JVM arguments and version metadata.
    if existing.iter().any(|arg| {
        arg.strip_prefix("-XX:+")
            .or_else(|| arg.strip_prefix("-XX:-"))
            .is_some_and(|flag| {
                [
                    "UseSerialGC",
                    "UseParallelGC",
                    "UseParallelOldGC",
                    "UseConcMarkSweepGC",
                    "UseParNewGC",
                    "UseG1GC",
                    "UseZGC",
                    "UseShenandoahGC",
                    "UseEpsilonGC",
                    "ZGenerational",
                ]
                .contains(&flag)
            })
    }) {
        return Ok(Vec::new());
    }
    let mut candidates: Vec<&[&str]> = Vec::new();
    if matches!(mode, "preferZgc" | "preferGenerationalZgc")
        && java.major >= 11
        && matches!(java.architecture.as_str(), "x64" | "arm64")
    {
        if mode == "preferGenerationalZgc" && (21..=23).contains(&java.major) {
            candidates.push(&["-XX:+UseZGC", "-XX:+ZGenerational"]);
        }
        // JDK 23 defaults to generational ZGC; JDK 24 removes the old collector and flag.
        candidates.push(match java.major {
            11..=14 => &["-XX:+UnlockExperimentalVMOptions", "-XX:+UseZGC"],
            23 => &["-XX:+UseZGC", "-XX:-ZGenerational"],
            _ => &["-XX:+UseZGC"],
        });
    }
    for flags in candidates {
        if supports(flags)? {
            return Ok(flags.iter().map(|s| (*s).into()).collect());
        }
    }
    Ok(if mode == "tunedG1" { TUNED_G1 } else { G1 }
        .iter()
        .map(|s| (*s).into())
        .collect())
}

pub fn probe(java: &Java, directory: &Path, flags: &[&str]) -> Result<bool> {
    // A short JVM launch also checks the actual vendor, OS and CPU support.
    let args = flags
        .iter()
        .copied()
        .chain(["-Xms16M", "-Xmx64M", "-version"])
        .map(String::from)
        .collect();
    let child = Child::spawn(&java.path, args, directory)?;
    let started = Instant::now();
    let result = loop {
        match child.poll() {
            Ok((status, _, _)) => {
                if let Some(code) = status["exitCode"].as_i64() {
                    break Ok(code == 0);
                }
            }
            Err(error) => break Err(error),
        }
        if started.elapsed() >= Duration::from_secs(5) {
            break Ok(false);
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    child.kill()?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collectors_follow_java_support_and_preserve_explicit_choices() {
        let mut java = Java {
            path: String::new(),
            version: String::new(),
            major: 8,
            architecture: "x64".into(),
        };
        for major in [8, 17, 21, 22, 23, 24, 25] {
            java.major = major;
            let flags = arguments("preferGenerationalZgc", &java, &[], |_| Ok(true)).unwrap();
            assert_eq!(
                flags,
                match major {
                    8 => vec!["-XX:+UseG1GC"],
                    21..=23 => vec!["-XX:+UseZGC", "-XX:+ZGenerational"],
                    _ => vec!["-XX:+UseZGC"],
                }
            );
        }
        java.major = 11;
        assert_eq!(
            arguments("preferZgc", &java, &[], |_| Ok(true)).unwrap(),
            ["-XX:+UnlockExperimentalVMOptions", "-XX:+UseZGC"]
        );
        java.major = 23;
        assert_eq!(
            arguments("preferZgc", &java, &[], |_| Ok(true)).unwrap(),
            ["-XX:+UseZGC", "-XX:-ZGenerational"]
        );
        java.major = 21;
        let mut attempts = Vec::new();
        assert_eq!(
            arguments("preferGenerationalZgc", &java, &[], |flags| {
                attempts.push(flags.iter().map(|s| (*s).to_string()).collect::<Vec<_>>());
                Ok(flags.len() == 1)
            })
            .unwrap(),
            ["-XX:+UseZGC"]
        );
        assert_eq!(
            attempts,
            [
                vec!["-XX:+UseZGC", "-XX:+ZGenerational"],
                vec!["-XX:+UseZGC"]
            ]
        );
        assert_eq!(
            arguments("preferGenerationalZgc", &java, &[], |_| Ok(false)).unwrap(),
            G1
        );
        java.architecture = "x86".into();
        assert_eq!(
            arguments("preferZgc", &java, &[], |_| panic!("32-bit ZGC probe")).unwrap(),
            G1
        );
        for mode in ["g1", "tunedG1"] {
            let flags =
                arguments(mode, &java, &[], |_| panic!("G1 does not need a probe")).unwrap();
            assert_eq!(flags, if mode == "g1" { G1 } else { TUNED_G1 });
        }
        for flag in [
            "-XX:+UseG1GC",
            "-XX:+UseZGC",
            "-XX:+UseParallelGC",
            "-XX:-UseG1GC",
            "-XX:+ZGenerational",
        ] {
            assert!(
                arguments("tunedG1", &java, &[flag.into()], |_| panic!())
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(arguments("invalid", &java, &[], |_| panic!()).is_err());
        assert_eq!(
            arguments(
                "g1",
                &java,
                &["-XX:+UseMaximumCompactionOnSystemGC".into()],
                |_| panic!()
            )
            .unwrap(),
            G1
        );
    }
}
