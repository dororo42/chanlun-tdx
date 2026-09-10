#!/usr/bin/env python
# -*- coding: utf-8 -*-
"""chanlun-tdx 提交前静态校验：Rust 括号平衡 / 公式结构 / mark 越界 / 忽略规则。"""
import re
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
FAIL = []


def check_rust_balance():
    print("=== 1) Rust 括号平衡 ===")
    for p in ["src/lib.rs", "src/chan.rs"]:
        s = (ROOT / p).read_text(encoding="utf-8")
        s = re.sub(r"//[^\n]*", "", s)
        s = re.sub(r"/\*.*?\*/", "", s, flags=re.S)
        s = re.sub(r'"(?:\\.|[^"\\])*"', '""', s)
        s = re.sub(r"'(?:\\.|[^'\\])'", "''", s)
        for a, b in (("{", "}"), ("(", ")"), ("[", "]")):
            if s.count(a) != s.count(b):
                FAIL.append(f"{p} {a}{b} 不平衡 {s.count(a)}/{s.count(b)}")
        print(f"  {p}: braces={s.count('{')} parens={s.count('(')}")


def check_formulas():
    print("=== 2) 公式结构（注释不嵌套 / 语句以 ; 结尾）===")
    for f in sorted((ROOT / "formulas").glob("*.txt")):
        t = f.read_text(encoding="utf-8")
        depth = mx = 0
        for ch in t:
            if ch == "{":
                depth += 1
                mx = max(mx, depth)
            elif ch == "}":
                depth -= 1
        body = re.sub(r"\{[^}]*\}", "", t)
        bad = [l for l in body.splitlines() if l.strip() and not l.strip().endswith(";")]
        status = "OK" if (mx <= 1 and not bad) else "FAIL"
        if status == "FAIL":
            FAIL.append(f"{f.name} 嵌套深度={mx} 未以;结尾={len(bad)}")
        print(f"  {f.name}: 嵌套深度={mx}(须<=1) 未以;结尾={len(bad)} -> {status}")


def check_marks():
    print("=== 3) 公式调用 mark vs DLL 注册表 ===")
    src = (ROOT / "src/lib.rs").read_text(encoding="utf-8")
    # 只取有实现函数的项；n_func_mark: 0 / None 是数组终止哨兵，不算功能 mark
    pairs = re.findall(r"n_func_mark: (\d+), p_call_func: Some", src)
    reg = {int(x) for x in pairs}
    print(f"  DLL 注册: {sorted(reg)}")
    if reg != set(range(1, 21)):
        FAIL.append(f"DLL 注册 mark 异常: {sorted(reg)} (期望 1..20)")
    if "n_func_mark: 0, p_call_func: None" not in src:
        FAIL.append("缺少终止哨兵项 n_func_mark: 0 / None")
    for f in sorted((ROOT / "formulas").glob("*.txt")):
        t = re.sub(r"\{[^}]*\}", "", f.read_text(encoding="utf-8"))
        used = sorted({int(x) for x in re.findall(r"TDXDLL1\(\s*(\d+)", t)})
        over = [u for u in used if u not in reg]
        if over:
            FAIL.append(f"{f.name} mark 越界: {over}")
        print(f"  {f.name}: mark={used} 越界={'无' if not over else over}")


def check_var_defs():
    print("=== 4) 公式变量定义完整性 ===")
    ID = r"[A-Za-z_\u4e00-\u9fff][A-Za-z0-9_\u4e00-\u9fff]*"
    BUILTIN = set("""TDXDLL1 TDXDLL2 TDXDLL3 TDXDLL4 HIGH LOW CLOSE OPEN VOL V C O H L
REF REFX BARSLAST BARSCOUNT SUM COUNT HHV LLV MA EMA SMA ABS MAX MIN IF IFF DRAWNULL
DRAWICON DRAWTEXT DRAWLINE STICKLINE POLYLINE DRAWBAND NAMELIKE AND OR NOT
COLORRED COLORGREEN COLORBROWN COLORYELLOW COLORMAGENTA COLORLIBLUE COLORWHITE COLORGRAY
COLOR404040 COLORFF8000 DOTLINE LINETHICK1 LINETHICK2 LINETHICK3 LINETHICK4 LINETHICK5
NODRAW CIRCLEDOT STICK VOLUME FINANCE DYNAINFO INBLOCK STRFIND BETWEEN EXIST EVERY
RANGE CROSS LONGCROSS TFILTER FILTER BACKSET""".split())
    for f in sorted((ROOT / "formulas").glob("*.txt")):
        t = re.sub(r"\{[^}]*\}", "", f.read_text(encoding="utf-8"))
        t = re.sub(r"'[^']*'", "''", t)
        defs = set(re.findall(r"(?:^|;)\s*(" + ID + r")\s*:=", t, re.M))
        outs = set(re.findall(r"(?:^|;)\s*(" + ID + r")\s*:", t, re.M))
        defined = defs | outs
        used = set(re.findall(ID, t))
        unknown = sorted(x for x in used if x not in defined and x not in BUILTIN)
        if unknown:
            FAIL.append(f"{f.name} 未定义标识符: {unknown}")
        print(f"  {f.name}: 定义 {len(defined)} 个, 未定义={'无' if not unknown else unknown}")


def check_ignore():
    print("=== 5) 忽略规则（子进程调用 git check-ignore）===")
    import subprocess

    def ignored(rel):
        r = subprocess.run(
            ["git", "check-ignore", "-q", rel], cwd=str(ROOT),
            capture_output=True,
        )
        return r.returncode == 0

    must_ignore = [
        "PLAN.md", "docs/formulas.md", "docs/semantics.md",
        "docs/dll-usage.md", "REPORT.md", "overview.md",
        "handoffs/x.md", "评估报告.md", "投顾分析报告.md",
    ]
    must_keep = [
        "src/lib.rs", "src/chan.rs", "README.md", "AGENTS.md",
        "Cargo.toml", "tests/parity.rs", ".github/workflows/build.yml",
        "formulas/缠论主图显示.txt", "formulas/缠论条件选股.txt",
        "formulas/缠论力度副图.txt",
    ]
    for x in must_ignore:
        if not ignored(x):
            FAIL.append(f"应忽略但未忽略: {x}")
        print(f"  忽略 {x}: {'OK' if ignored(x) else 'FAIL'}")
    for x in must_keep:
        if ignored(x):
            FAIL.append(f"被误忽略: {x}")
        print(f"  保留 {x}: {'FAIL 被误忽略' if ignored(x) else 'OK'}")


def check_no_report_tracked():
    print("=== 6) 版本库内不得存在报告类文件 ===")
    import subprocess

    r = subprocess.run(["git", "ls-files"], cwd=str(ROOT), capture_output=True, text=True)
    tracked = [l.strip() for l in r.stdout.splitlines() if l.strip()]
    bad = [
        f for f in tracked
        if re.search(r"(report|报告|handoffs|overview)", f, re.I) or f.startswith(("docs/", "PLAN"))
    ]
    if bad:
        FAIL.append(f"版本库内仍有报告/规划文件: {bad}")
    print(f"  跟踪文件总数={len(tracked)}, 疑似报告/规划文件={'无' if not bad else bad}")


if __name__ == "__main__":
    check_rust_balance()
    check_formulas()
    check_marks()
    check_var_defs()
    check_ignore()
    check_no_report_tracked()
    print()
    if FAIL:
        print("结果: FAIL")
        for x in FAIL:
            print("  -", x)
        sys.exit(1)
    print("结果: 全部通过 ✓")
