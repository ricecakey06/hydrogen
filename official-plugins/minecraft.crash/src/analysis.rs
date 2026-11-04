use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cause {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub suggestions: Vec<String>,
    pub evidence: Vec<String>,
}

struct Rule {
    id: &'static str,
    title: &'static str,
    summary: &'static str,
    patterns: &'static [&'static str],
    suggestions: &'static [&'static str],
}

const RULES: &[Rule] = &[
    Rule {
        id: "heap-memory",
        title: "游戏可用内存不足",
        summary: "日志出现 Java 堆内存耗尽的记录。",
        patterns: &[
            "outofmemoryerror: java heap space",
            "outofmemoryerror: gc overhead",
        ],
        suggestions: &[
            "在版本设置中适当增加游戏内存，并为系统保留足够内存。",
            "降低资源包分辨率、渲染距离或模组数量后重试。",
        ],
    },
    Rule {
        id: "metaspace",
        title: "Java 元空间不足",
        summary: "用于加载类的元空间耗尽，需检查 JVM 限制或模组加载情况。",
        patterns: &[
            "outofmemoryerror: metaspace",
            "outofmemoryerror: compressed class space",
        ],
        suggestions: &[
            "检查自定义 JVM 参数中的 MaxMetaspaceSize 等限制，尝试恢复默认参数。",
            "检查最近添加的模组及加载器版本，更新或暂时禁用异常模组后重试。",
        ],
    },
    Rule {
        id: "native-memory",
        title: "系统或原生内存不足",
        summary: "Java 无法继续申请系统内存，或无法保留指定大小的堆空间。",
        patterns: &[
            "there is insufficient memory for the java runtime",
            "native memory allocation (malloc) failed",
            "could not reserve enough space",
            "unable to create native thread",
            "outofmemoryerror: direct buffer memory",
        ],
        suggestions: &[
            "关闭占用内存较多的程序，检查系统可用内存与分页文件。",
            "若设置了过大的游戏内存，请适当调低；确认使用 64 位 Java。",
        ],
    },
    Rule {
        id: "java-version",
        title: "Java 版本与游戏或模组不兼容",
        summary: "日志包含不支持的 Java 字节码版本。",
        patterns: &[
            "unsupportedclassversionerror",
            "compiled by a more recent version of the java runtime",
            "unsupported class file major version",
        ],
        suggestions: &[
            "在版本设置中选择符合当前 Minecraft 和模组要求的 Java。",
            "检查最近添加的模组是否要求更高版本的 Java 或不同的游戏版本。",
        ],
    },
    Rule {
        id: "graphics",
        title: "显卡驱动或 OpenGL 环境异常",
        summary: "日志记录了图形上下文、驱动或 OpenGL 初始化失败。",
        patterns: &[
            "glfw error 65542",
            "glfw error 65543",
            "glfw error 65545",
            "wgl: the driver does not appear to support opengl",
            "failed to create window",
            "pixel format not accelerated",
            "opengl is not supported",
        ],
        suggestions: &[
            "从显卡厂商安装适用于当前系统的驱动，然后重启电脑。",
            "尝试关闭光影、图形增强模组和叠加层；双显卡设备可为 Java 指定高性能显卡。",
        ],
    },
    Rule {
        id: "mod-dependencies",
        title: "模组依赖缺失或版本冲突",
        summary: "模组加载器检测到了不兼容的模组集合或未满足的依赖。",
        patterns: &[
            "incompatible mod set",
            "incompatible mods found",
            "mod resolution encountered an incompatible mod set",
            "modresolutionexception",
            "missing mandatory dependencies",
            "missing or unsupported mandatory dependencies",
            "duplicate mods",
            "duplicatemodsexception",
            "duplicatemodsfoundexception",
            "duplicate versions for mod id",
        ],
        suggestions: &[
            "查看日志中标注的模组名称和所需版本，补齐依赖或安装兼容版本。",
            "确认模组对应当前游戏版本及 Fabric、Forge 或 NeoForge 加载器，移除重复模组。",
        ],
    },
    Rule {
        id: "mixin",
        title: "模组注入失败或相互冲突",
        summary: "日志出现 Mixin 注入或应用失败，可能与模组版本或组合有关。",
        patterns: &[
            "mixinapplyerror",
            "mixintransformererror",
            "invalidmixinexception",
            "injectionerror",
            "critical injection failure",
            "mixin apply failed",
        ],
        suggestions: &[
            "优先检查错误附近提到的模组，更新到适用于当前游戏和加载器的版本。",
            "备份后暂时禁用最近添加的模组，逐批恢复以定位冲突。",
        ],
    },
    Rule {
        id: "missing-library",
        title: "运行库缺失或组件版本不匹配",
        summary: "Java 未能找到所需类、方法或原生运行库。",
        patterns: &[
            "noclassdeffounderror",
            "classnotfoundexception",
            "nosuchmethoderror",
            "unsatisfiedlinkerror",
            "could not find or load main class",
        ],
        suggestions: &[
            "在版本设置中校验或修复游戏文件，然后重试。",
            "如果错误指向某个模组，检查它的依赖、游戏版本与加载器版本。",
        ],
    },
    Rule {
        id: "jvm-arguments",
        title: "Java 启动参数无效",
        summary: "Java 拒绝了当前启动参数或堆内存配置。",
        patterns: &[
            "unrecognized vm option",
            "unrecognized option:",
            "improperly specified vm option",
            "invalid maximum heap size",
            "initial heap size set to a larger value than the maximum heap size",
        ],
        suggestions: &[
            "移除最近添加的自定义 JVM 参数，使用默认配置重试。",
            "检查参数是否适用于当前 Java 版本，以及最小内存是否超过最大内存。",
        ],
    },
    Rule {
        id: "native-crash",
        title: "Java 或原生组件发生崩溃",
        summary: "日志记录了原生异常，需结合 Problematic frame 等信息进一步定位。",
        patterns: &[
            "a fatal error has been detected by the java runtime",
            "exception_access_violation",
            "sigsegv",
            "sigbus",
        ],
        suggestions: &[
            "检查 JVM 错误日志中的 Problematic frame；若指向显卡驱动，先更新驱动并关闭光影。",
            "尝试相同主版本的较新 Java，并暂时禁用包含原生代码的模组或叠加层。",
        ],
    },
];

pub fn analyze(logs: &[String]) -> Vec<Cause> {
    let mut causes = Vec::new();
    for rule in RULES {
        let evidence: Vec<_> = logs
            .iter()
            .flat_map(|log| log.lines())
            .filter(|line| {
                let lower = line.to_ascii_lowercase();
                rule.patterns.iter().any(|pattern| lower.contains(pattern))
            })
            .take(3)
            .map(|line| line.chars().take(240).collect::<String>())
            .collect();
        if !evidence.is_empty() {
            causes.push(Cause {
                id: rule.id.into(),
                title: rule.title.into(),
                summary: rule.summary.into(),
                suggestions: rule.suggestions.iter().map(|s| (*s).into()).collect(),
                evidence,
            });
        }
        if causes.len() == 4 {
            break;
        }
    }
    if causes.is_empty() {
        causes.push(Cause {
            id: "unknown".into(),
            title: "暂未识别出具体原因".into(),
            summary: "退出码只能说明游戏异常结束，现有日志不足以确定原因。".into(),
            suggestions: vec![
                "打开崩溃日志，检查末尾异常及其 Caused by 信息。".into(),
                "回顾最近更改的模组、资源包和 Java 设置；需要协助时可导出报告。".into(),
            ],
            evidence: vec![],
        });
    }
    causes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_heap_and_system_memory_advice() {
        assert_eq!(
            analyze(&["java.lang.OutOfMemoryError: Java heap space".into()])[0].id,
            "heap-memory"
        );
        let native = analyze(&["Could not reserve enough space for 2097152KB object heap".into()]);
        assert_eq!(native[0].id, "native-memory");
        assert!(native[0].suggestions.iter().any(|s| s.contains("调低")));
    }
    #[test]
    fn recognizes_multiple_causes_with_bounded_evidence() {
        let log = "UnsupportedClassVersionError\nMixinApplyError\nGLFW error 65542\n".repeat(20);
        let causes = analyze(&[log]);
        assert!(causes.iter().any(|c| c.id == "java-version"));
        assert!(causes.iter().any(|c| c.id == "mixin"));
        assert!(causes.iter().any(|c| c.id == "graphics"));
        assert!(causes.iter().all(|c| c.evidence.len() <= 3));
    }
    #[test]
    fn does_not_diagnose_from_an_exit_code_or_generic_error() {
        assert_eq!(analyze(&["ERROR: game exited: -1".into()])[0].id, "unknown");
        assert_eq!(analyze(&[])[0].id, "unknown");
        assert_eq!(
            analyze(&["INFO: this package requires version 1.0".into()])[0].id,
            "unknown"
        );
    }
    #[test]
    fn metaspace_failure_does_not_suggest_increasing_the_game_heap() {
        let causes = analyze(&["java.lang.OutOfMemoryError: Metaspace".into()]);
        assert_eq!(causes[0].id, "metaspace");
        assert!(causes[0].suggestions[0].contains("MaxMetaspaceSize"));
        assert!(!causes.iter().any(|cause| cause.id == "heap-memory"));
    }
}
