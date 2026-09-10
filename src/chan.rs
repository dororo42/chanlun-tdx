//! chan.rs — 缠论核心（移植自 YuYuKunKun/chanlun.py，口径见 docs/semantics.md）。
//!
//! 批处理实现，以 tests/fixtures/parity_*.json（Python 参考实现输出）为对齐契约：
//! K线合并 → 分型 → 笔 → 笔中枢 → 线段 → 线段中枢 → MACD → 买卖点。
//! 各函数注释标注 chan.py 对应行号。

/// 相对方向（chan.py L1115-1147 `相对方向.分析`）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rel {
    Same,
    Down,
    DownGap,
    DownTouch,
    Up,
    UpGap,
    UpTouch,
    Shun,
    Ni,
}

impl Rel {
    pub fn is_up(self) -> bool {
        matches!(self, Rel::Up | Rel::UpGap | Rel::UpTouch)
    }
    pub fn is_down(self) -> bool {
        matches!(self, Rel::Down | Rel::DownGap | Rel::DownTouch)
    }
    pub fn is_contain(self) -> bool {
        matches!(self, Rel::Shun | Rel::Ni | Rel::Same)
    }
    pub fn is_gap(self) -> bool {
        matches!(self, Rel::UpGap | Rel::DownGap)
    }
}

pub fn rel(ph: f64, pl: f64, ch: f64, cl: f64) -> Rel {
    if ph == ch && pl == cl {
        return Rel::Same;
    }
    if ph > ch && pl > cl {
        if pl == ch {
            Rel::DownTouch
        } else if pl > ch {
            Rel::DownGap
        } else {
            Rel::Down
        }
    } else if ph < ch && pl < cl {
        if ph == cl {
            Rel::UpTouch
        } else if ph < cl {
            Rel::UpGap
        } else {
            Rel::Up
        }
    } else if ph >= ch && pl <= cl {
        Rel::Shun
    } else if ph <= ch && pl >= cl {
        Rel::Ni
    } else {
        // NaN/Infinity 或异常数据 → 视为同向（安全降级，不 panic）
        Rel::Same
    }
}

/// 原始 K 线
#[derive(Clone, Copy, Debug)]
pub struct Bar {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
}

/// 分型结构（chan.py L1168）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Up,
    Down,
    Top,
    Bottom,
}

/// 缠论K线（chan.py L3199）
#[derive(Clone, Copy, Debug)]
pub struct ChanK {
    /// 原始起始序号
    pub raw_start: usize,
    /// 原始结束序号（每次合并都更新，chan.py `原始结束序号`）
    pub raw_end: usize,
    /// 标的K线序号（仅非"顺"合并时更新，chan.py `标的K线.序号`；
    /// 分型标记与笔/线段端点位置都用它）
    pub mark_bar: usize,
    pub high: f64,
    pub low: f64,
    /// 分型标记终态（含流式语义的右端临时标记）
    pub kind: Option<Kind>,
    /// 分型特征值（顶=高，底=低）
    pub feat: f64,
}

/// 三根缠K的分型判定（chan.py L1191，默认 可以逆序包含=False）
fn triple_kind(l: &ChanK, m: &ChanK, r: &ChanK) -> Option<Kind> {
    let lr = rel(l.high, l.low, m.high, m.low);
    let rr = rel(m.high, m.low, r.high, r.low);
    if lr.is_contain() || rr.is_contain() {
        return None;
    }
    match (lr.is_up(), rr.is_up()) {
        (true, true) => Some(Kind::Up),
        (true, false) => Some(Kind::Top),
        (false, true) => Some(Kind::Bottom),
        (false, false) => Some(Kind::Down),
    }
}

/// K线合并（chan.py L3382 `_兼并`）+ 分型标记终态（L3486-3517）
pub fn merge_and_mark(bars: &[Bar]) -> Vec<ChanK> {
    let mut ks: Vec<ChanK> = Vec::with_capacity(bars.len() / 2 + 8);
    for (i, b) in bars.iter().enumerate() {
        if ks.is_empty() {
            ks.push(ChanK {
                raw_start: i,
                raw_end: i,
                mark_bar: i,
                high: b.high,
                low: b.low,
                kind: None,
                feat: 0.0,
            });
            continue;
        }
        let last = ks.len() - 1;
        let r = rel(ks[last].high, ks[last].low, b.high, b.low);
        if r.is_contain() {
            // 合并方向：前前缠K vs 当前缠K（L3417）
            let take_min =
                ks.len() >= 2 && rel(ks[last - 1].high, ks[last - 1].low, ks[last].high, ks[last].low).is_down();
            if take_min {
                ks[last].high = ks[last].high.min(b.high);
                ks[last].low = ks[last].low.min(b.low);
            } else {
                ks[last].high = ks[last].high.max(b.high);
                ks[last].low = ks[last].low.max(b.low);
            }
            // 原始结束序号总是更新；标的K线仅非"顺"时更新（L3420-3425）
            ks[last].raw_end = i;
            if r != Rel::Shun {
                ks[last].mark_bar = i;
            }
        } else {
            let kind = if r.is_down() { Kind::Down } else { Kind::Up };
            ks.push(ChanK {
                raw_start: i,
                raw_end: i,
                mark_bar: i,
                high: b.high,
                low: b.low,
                kind: Some(kind),
                feat: 0.0,
            });
        }
    }
    // 分型标记：逐三元组升序覆盖；右端临时标记规则与流式终态一致（L3494-3512）
    if ks.len() >= 3 {
        for i in 1..ks.len() - 1 {
            let (l, m, r) = (ks[i - 1], ks[i], ks[i + 1]);
            if let Some(k) = triple_kind(&l, &m, &r) {
                ks[i].kind = Some(k);
                ks[i].feat = match k {
                    Kind::Top | Kind::Up => m.high,
                    Kind::Bottom | Kind::Down => m.low,
                };
                let (rk, rf) = match k {
                    Kind::Bottom | Kind::Up => (Kind::Top, r.high),
                    Kind::Top | Kind::Down => (Kind::Bottom, r.low),
                };
                ks[i + 1].kind = Some(rk);
                ks[i + 1].feat = rf;
            }
        }
    }
    ks
}

/// 确认分型（作为"中"被判定的顶/底），供笔使用
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fx {
    /// 缠K序号
    pub k: usize,
    pub top: bool,
    pub feat: f64,
}

pub fn collect_fractals(ks: &[ChanK]) -> Vec<Fx> {
    let mut out = Vec::new();
    if ks.len() < 3 {
        return out;
    }
    for i in 1..ks.len() - 1 {
        match ks[i].kind {
            Some(Kind::Top) => out.push(Fx { k: i, top: true, feat: ks[i].feat }),
            Some(Kind::Bottom) => out.push(Fx { k: i, top: false, feat: ks[i].feat }),
            _ => {}
        }
    }
    // 右端临时分型（chan.py L3504-3517：上/下链会给最后一根缠K赋右端临时标记；
    // L4859-4862 的"跳右"递归可从右端构造分型(左,中,None)，驱动尾笔（有效性=False））
    let last = ks.len() - 1;
    let tmp = match ks[last].kind {
        Some(Kind::Top) => Some(Fx { k: last, top: true, feat: ks[last].feat }),
        Some(Kind::Bottom) => Some(Fx { k: last, top: false, feat: ks[last].feat }),
        _ => None,
    };
    if let Some(f) = tmp {
        if out.last().map(|e| e.k) != Some(f.k) {
            out.push(f);
        }
    }
    out
}

/// [a,b] 缠K区间内实际最高(top)/最低(bottom) 的缠K序号；
/// tie_latest=false 同值取最早（chan.py `_实际高点/_实际低点` 取舍=False）
fn extreme_k(ks: &[ChanK], a: usize, b: usize, top: bool, tie_latest: bool) -> usize {
    let mut best = a;
    for i in a..=b {
        let (vi, vb) = if top { (ks[i].high, ks[best].high) } else { (ks[i].low, ks[best].low) };
        let better = if top { vi > vb } else { vi < vb };
        if better || (equal_f(vi, vb) && tie_latest) {
            best = i;
        }
    }
    best
}

fn equal_f(a: f64, b: f64) -> bool {
    a == b
}

/// 笔端点划分（chan.py L4772 `笔.分析` 批处理等价；默认配置：
/// 笔内元素数量=5、笔弱化=False、笔次级成笔=False、笔内相同终点取舍=False）
pub fn build_strokes(ks: &[ChanK], fxs: &[Fx]) -> Vec<Fx> {
    if fxs.is_empty() {
        return vec![];
    }
    const MIN_K: usize = 5;
    let mut eps: Vec<Fx> = Vec::new();
    let pos = |k: usize| fxs.iter().position(|x| x.k == k);
    let mut guard = 0usize; // 防御性迭代上限
    let mut i = 0usize;
    while i < fxs.len() {
        guard += 1;
        if guard > 2_000_000 {
            break; // 迭代超限 → 安全退出（不 panic）
        }
        let f = fxs[i];
        match eps.last().copied() {
            None => {
                eps.push(f);
                i += 1;
            }
            Some(l) if l.top != f.top => {
                let span = f.k - l.k + 1;
                let mut handled = false;
                if span >= MIN_K {
                    // 起点修正（L4826-4839 文官）：同型实际极值不是 l 时，
                    // 更极值 → 弹出并从"武将"起重放；同值 → 忽略（与流式一致），随后做终点校验
                    let start_ext = extreme_k(ks, l.k, f.k, l.top, false);
                    if start_ext != l.k {
                        if let Some(si) = pos(start_ext) {
                            let se = fxs[si];
                            let more = if l.top { se.feat > l.feat } else { se.feat < l.feat };
                            if more {
                                eps.pop();
                                let wj = extreme_k(ks, l.k, start_ext, !l.top, false);
                                i = fxs.iter().position(|x| x.k >= wj).unwrap_or(fxs.len());
                                continue;
                            }
                        }
                    }
                    // 终点校验（L4841-4848 武将）：当前分型须为区间实际反极值（同值取最早）
                    let end_ext = extreme_k(ks, l.k, f.k, f.top, false);
                    let dir_ok = {
                        let rr = rel(ks[l.k].high, ks[l.k].low, ks[f.k].high, ks[f.k].low);
                        if f.top { rr.is_up() } else { rr.is_down() }
                    };
                    if end_ext == f.k && dir_ok {
                        eps.push(f);
                        handled = true;
                    }
                }
                if !handled {
                    // 不成笔：丢弃当前分型，跳到其"右"分型（L4859-4862 跳右递归的批处理等价）
                    let wj = f.k + 2; // 右 = 中.序号+1 的缠K 再 +1（右分型的中）
                    i = fxs.iter().position(|x| x.k >= wj).unwrap_or(fxs.len());
                    continue;
                }
                i += 1;
            }
            Some(l) => {
                // 同型（L4864-4888）：更极值 → 弹出，从区间反极值（武将）起重放；否则忽略
                let more = if l.top { f.feat > l.feat } else { f.feat < l.feat };
                if more {
                    eps.pop();
                    if eps.is_empty() {
                        // 序列已空：直接添加当前分型（L4889-4890）
                        eps.push(f);
                        i += 1;
                    } else {
                        let wj = extreme_k(ks, l.k, f.k, !l.top, false);
                        i = fxs.iter().position(|x| x.k >= wj).unwrap_or(fxs.len());
                    }
                } else {
                    i += 1;
                }
            }
        }
    }
    eps
}

/// 虚线（笔/线段共用；chan.py L3770 `虚线`）
#[derive(Clone, Copy, Debug)]
pub struct Line {
    /// 文端原始 bar（文.中.标的K线.序号 = 缠K.raw_end）
    pub wen: usize,
    /// 武端原始 bar
    pub wu: usize,
    pub dir_up: bool,
    pub high: f64,
    pub low: f64,
    pub wen_feat: f64,
    pub wu_feat: f64,
}

pub fn strokes_to_lines(ks: &[ChanK], eps: &[Fx]) -> Vec<Line> {
    let mut out = Vec::new();
    for w in eps.windows(2) {
        let (a, b) = (w[0], w[1]);
        out.push(Line {
            wen: ks[a.k].mark_bar,
            wu: ks[b.k].mark_bar,
            dir_up: b.top,
            high: a.feat.max(b.feat),
            low: a.feat.min(b.feat),
            wen_feat: a.feat,
            wu_feat: b.feat,
        });
    }
    out
}

/// 中枢（chan.py L6513 `中枢` + L6889 `中枢.分析`）
#[derive(Clone, Debug)]
pub struct Pivot {
    /// 上沿 = 前三线高点最小值
    pub zg: f64,
    /// 下沿 = 前三线低点最大值
    pub zd: f64,
    /// 基础序列的线下标区间 [start_li, end_li]（闭）
    pub start_li: usize,
    pub end_li: usize,
    /// 第三买卖线（线下标）
    pub tbs: Option<usize>,
}

fn pivot_zg_zd(lines: &[Line], a: usize) -> (f64, f64) {
    let zg = lines[a].high.min(lines[a + 1].high).min(lines[a + 2].high);
    let zd = lines[a].low.max(lines[a + 1].low).max(lines[a + 2].low);
    (zg, zd)
}

/// 基础检查（L6803）：左vs右 关系 ∈ {向上,向下,顺,逆,同}（排除缺口与衔接）
fn pivot_base_ok(lines: &[Line], a: usize) -> bool {
    matches!(
        rel(lines[a].high, lines[a].low, lines[a + 2].high, lines[a + 2].low),
        Rel::Up | Rel::Down | Rel::Shun | Rel::Ni | Rel::Same
    )
}

/// 中枢批处理主流程（L6889-6955）
pub fn build_pivots(lines: &[Line]) -> Vec<Pivot> {
    let n = lines.len();
    if n < 3 {
        return vec![];
    }
    // 1) 首个中枢（L6904-6922）
    let mut first: Option<(usize, f64, f64)> = None;
    'outer: for i in 1..n - 1 {
        if !pivot_base_ok(lines, i - 1) {
            continue;
        }
        if i - 1 == 0 {
            continue; // 跳过首部
        }
        if i >= 2 {
            // 同向进入跳过（L6912-6917）
            let r = rel(lines[i - 2].high, lines[i - 2].low, lines[i - 1].high, lines[i - 1].low);
            if (r.is_up() && lines[i - 1].dir_up) || (r.is_down() && !lines[i - 1].dir_up) {
                continue 'outer;
            }
        }
        let (zg, zd) = pivot_zg_zd(lines, i - 1);
        first = Some((i - 1, zg, zd));
        break;
    }
    let (mut cur_start, mut cur_zg, mut cur_zd) = match first {
        Some(x) => x,
        None => return vec![],
    };
    let mut cur_end = cur_start + 2;
    let mut pivots: Vec<Pivot> = Vec::new();
    let mut buf: Vec<usize> = Vec::new();
    let mut tbs: Option<usize> = None;
    let mut j = cur_end + 1;
    while j < n {
        let l = lines[j];
        let r = rel(cur_zg, cur_zd, l.high, l.low);
        if r.is_gap() {
            // 缺口 → 缓冲；与中枢末线连续的首条缺口线 = 第三买卖线（L6934-6939）
            if tbs.is_none() && buf.is_empty() && j == cur_end + 1 {
                tbs = Some(j);
            }
            buf.push(j);
        } else if buf.is_empty() {
            cur_end = j; // 延伸（L6943）
        } else {
            buf.push(j);
        }
        // 缓冲成枢（L6947-6954）
        while buf.len() >= 3 {
            let want_up = !lines[cur_end].dir_up;
            let mut found = None;
            for t in 1..buf.len() - 1 {
                let a = buf[t - 1];
                if !pivot_base_ok(lines, a) {
                    continue;
                }
                if lines[a].dir_up == want_up {
                    found = Some(a);
                    break;
                }
            }
            match found {
                Some(a) => {
                    pivots.push(Pivot { zg: cur_zg, zd: cur_zd, start_li: cur_start, end_li: cur_end, tbs });
                    let (zg, zd) = pivot_zg_zd(lines, a);
                    cur_start = a;
                    cur_end = a + 2;
                    cur_zg = zg;
                    cur_zd = zd;
                    tbs = None;
                    buf.clear(); // L6954
                }
                None => {
                    buf.remove(0);
                }
            }
        }
        j += 1;
    }
    pivots.push(Pivot { zg: cur_zg, zd: cur_zd, start_li: cur_start, end_li: cur_end, tbs });
    pivots
}

/// 特征元素（chan.py L4997 `线段特征`；高低取值口径 L5052-5098）
#[derive(Clone, Debug)]
struct FeatElem {
    /// 成员首/末笔下标（闭）
    first_bi: usize,
    last_bi: usize,
    high: f64,
    low: f64,
    /// 文端特征值与所在笔：向上段=簇内最高顶（同值取时间晚者 L5061）；向下段=最低底（同值取时间早者）
    wen_feat: f64,
    wen_bi: usize,
}

fn elem_from(bis: &[Line], dir_up: bool, members: &[usize]) -> FeatElem {
    // 向上段特征簇整体取 max（高低都取簇内最大），向下段整体取 min（chan.py L5052-5098：
    // 向上取高高中的最大 / 向下取低低中的最小）
    let (mut high, mut low) =
        if dir_up { (f64::NEG_INFINITY, f64::NEG_INFINITY) } else { (f64::INFINITY, f64::INFINITY) };
    let mut wen_feat = if dir_up { f64::NEG_INFINITY } else { f64::INFINITY };
    let mut wen_bi = members[0];
    for &bi in members {
        let b = &bis[bi];
        // dir_up 时成员是下行笔：起点=顶(wen_feat)、终点=底(wu_feat)；
        // !dir_up 时成员是上行笔：起点=底(wen_feat)、终点=顶(wu_feat)。
        let (start_feat, end_feat) = (b.wen_feat, b.wu_feat);
        if dir_up {
            high = high.max(start_feat);
            low = low.max(end_feat);
            if start_feat > wen_feat {
                wen_feat = start_feat;
                wen_bi = bi;
            }
        } else {
            high = high.min(end_feat);
            low = low.min(start_feat);
            if start_feat < wen_feat {
                wen_feat = start_feat;
                wen_bi = bi;
            }
        }
    }
    FeatElem { first_bi: members[0], last_bi: members[members.len() - 1], high, low, wen_feat, wen_bi }
}

/// 特征范围分型判定（L5162/5198：可以逆序包含=True，忽视顺序包含=True）
fn triple_kind_ranges(l: &FeatElem, m: &FeatElem, r: &FeatElem) -> Option<Kind> {
    let lr = rel(l.high, l.low, m.high, m.low);
    let rr = rel(m.high, m.low, r.high, r.low);
    match (lr, rr) {
        (a, b) if a.is_up() && b.is_up() => Some(Kind::Up),
        (a, b) if a.is_up() && b.is_down() => Some(Kind::Top),
        (a, b) if a.is_down() && b.is_up() => Some(Kind::Bottom),
        (a, b) if a.is_down() && b.is_down() => Some(Kind::Down),
        (Rel::Ni, b) if b.is_up() => Some(Kind::Bottom),
        (Rel::Ni, b) if b.is_down() => Some(Kind::Top),
        (a, Rel::Ni) if a.is_up() => Some(Kind::Up),
        (a, Rel::Ni) if a.is_down() => Some(Kind::Down),
        _ => None,
    }
}

/// 特征序列分析结果
struct FeatScan {
    feats: Vec<FeatElem>,
    /// 末三特征构成终止分型时的（终点笔, 下一特征左/中间隔缺口）
    terminated: Option<(usize, bool, usize)>,
}

/// 特征序列静态分析（L5135-5187）+ 逐笔终止检查（L5314-5330/L5419-5421）
/// 流式语义：每根笔处理（同向→可能分型替换；反向→合并/新建）后，
/// 立即检查末三特征是否构成终止分型（向上段=顶，向下段=底）。
fn scan_features(bis: &[Line], seg_bis: &[usize], dir_up: bool, prev_gap: bool) -> FeatScan {
    let mut feats: Vec<FeatElem> = Vec::new();
    let mut terminated: Option<(usize, bool, usize)> = None;
    let merge_ni = prev_gap; // 老阳/老阴 且未开"忽视"开关（L5145-5150）
    for &bi in seg_bis {
        let b = &bis[bi];
        if b.dir_up == dir_up {
            // 同向笔：分型替换检查（L5155-5172），特征序列<3 时直接跳过。
            // 替换吞掉末两特征 → 此前的终止判定失效（终止分型的"中/右"被吞）。
            if feats.len() >= 3 {
                let n = feats.len();
                let k = triple_kind_ranges(&feats[n - 3], &feats[n - 2], &feats[n - 1]);
                let hit = match (dir_up, k) {
                    (true, Some(Kind::Top)) => b.high > feats[n - 2].high,
                    (false, Some(Kind::Bottom)) => b.low < feats[n - 2].low,
                    _ => false,
                };
                if hit {
                    let n = feats.len();
                    let mid = feats[n - 2].clone();
                    let right = feats[n - 1].clone();
                    let (fh, fl) = fake_range(bis, dir_up, mid.first_bi, right.last_bi);
                    feats.pop();
                    feats.pop();
                    feats.push(FeatElem {
                        first_bi: mid.first_bi,
                        last_bi: right.last_bi,
                        high: fh,
                        low: fl,
                        wen_feat: fh,
                        wen_bi: mid.first_bi,
                    });
                    // 替换吞掉终止分型的中/右 → 撤销终止
                    if let Some((tbi, _, _)) = terminated {
                        if tbi >= mid.first_bi {
                            terminated = None;
                        }
                    }
                }
            }
        } else {
            // 反向笔 → 合并或新建特征（L5176-5185）
            match feats.last_mut() {
                None => feats.push(elem_from(bis, dir_up, &[bi])),
                Some(prev) => {
                    let r = rel(prev.high, prev.low, b.high, b.low);
                    if r == Rel::Shun || r == Rel::Same || (merge_ni && r == Rel::Ni) {
                        prev.last_bi = bi;
                        let (start_feat, end_feat) = (b.wen_feat, b.wu_feat);
                        if dir_up {
                            prev.high = prev.high.max(start_feat);
                            prev.low = prev.low.max(end_feat);
                            if start_feat > prev.wen_feat {
                                prev.wen_feat = start_feat;
                                prev.wen_bi = bi;
                            }
                        } else {
                            prev.high = prev.high.min(end_feat);
                            prev.low = prev.low.min(start_feat);
                            if start_feat < prev.wen_feat {
                                prev.wen_feat = start_feat;
                                prev.wen_bi = bi;
                            }
                        }
                    } else {
                        feats.push(elem_from(bis, dir_up, &[bi]));
                    }
                }
            }
            // 终止检查：反向笔处理后判定末三特征是否构成终止分型
            if terminated.is_none() && feats.len() >= 3 {
                let n = feats.len();
                let k = triple_kind_ranges(&feats[n - 3], &feats[n - 2], &feats[n - 1]);
                let hit = match (dir_up, k) {
                    (true, Some(Kind::Top)) => true,
                    (false, Some(Kind::Bottom)) => true,
                    _ => false,
                };
                if hit {
                    let gap = rel(feats[n - 3].high, feats[n - 3].low, feats[n - 2].high, feats[n - 2].low)
                        .is_gap();
                    terminated = Some((feats[n - 1].last_bi, gap, feats[n - 2].wen_bi));
                }
            }
            if terminated.is_some() {
                break;
            }
        }
    }
    FeatScan { feats, terminated }
}

/// fake 元素范围（L5166-5170）：小号.文 → 大号.武
fn fake_range(bis: &[Line], dir_up: bool, first_bi: usize, last_bi: usize) -> (f64, f64) {
    let start = if dir_up { bis[first_bi].wen_feat } else { bis[first_bi].wen_feat };
    let end = if dir_up { bis[last_bi].wu_feat } else { bis[last_bi].wu_feat };
    (start.max(end), start.min(end))
}

/// 线段（chan.py L5236 `线段`，批处理等价）
#[derive(Clone, Debug)]
pub struct Segment {
    pub dir_up: bool,
    /// 文/武端原始 bar 与特征值（武 = 终止特征中元素的文端）
    pub wen: usize,
    pub wu: usize,
    pub wen_feat: f64,
    pub wu_feat: f64,
    /// 包含的笔下标区间 [start_bi, end_bi]（闭）
    pub start_bi: usize,
    pub end_bi: usize,
}

pub fn build_segments(bis: &[Line]) -> Vec<Segment> {
    let mut segs: Vec<Segment> = Vec::new();
    let n = bis.len();
    if n < 3 {
        return segs;
    }
    // 首段创建（chan.py L5994-6004）：从 笔序列[1..n-2] 找首个满足 _基础判断 的三笔组
    // （连续、左中/中右均包含、左vs右关系∈{向上,向下} 且与左笔方向一致）
    let mut start = usize::MAX;
    for i in 1..n - 1 {
        let (l, m, r) = (&bis[i - 1], &bis[i], &bis[i + 1]);
        let lm = rel(l.high, l.low, m.high, m.low);
        let mr = rel(m.high, m.low, r.high, r.low);
        if !lm.is_contain() || !mr.is_contain() {
            continue;
        }
        let lr = rel(l.high, l.low, r.high, r.low);
        let ok = (l.dir_up && lr.is_up()) || (!l.dir_up && lr.is_down());
        if ok {
            start = i - 1;
            break;
        }
    }
    if start == usize::MAX {
        return segs;
    }
    let mut i = start; // 新段起始笔（= 上段终止"中"元素的文端笔，与上段共享端点 bar）
    let mut feat_from = start; // 特征扫描起点（L5414：前一结束位置-1）
    let mut prev_gap = false;
    // 前进性保险：i 每轮至少 +1（由 mid_wen > i 保证），正常最多 n 轮。
    // 无限循环会无限 push Segment → OOM abort，catch_unwind 拦不住（TDX 直接退出）。
    let mut loop_guard = 0usize;
    while i < n {
        loop_guard += 1;
        if loop_guard > n + 4 {
            break; // 理论不可达：防御异常数据形态
        }
        // 首段方向 = 首笔方向；后续段起点笔的方向即新段方向（L6076-6077 分割序列的后序列）
        let dir_up = bis[i].dir_up;
        let view: Vec<usize> = (feat_from..bis.len()).collect();
        let scan = scan_features(bis, &view, dir_up, prev_gap);
        match scan.terminated {
            // 终止笔为序列最后一笔时，其右特征不完整（右端临时分型所致），不构成有效终止；
            // mid_wen 必须 > i（前进性）：否则 i = mid_wen 后 view 不变 → 死循环。
            Some((end_bi, gap, mid_wen)) if end_bi > i && end_bi < n - 1 && mid_wen > i => {
                // 终止分型"中"元素的 bar（py: 武 = 特征分型"中".文.中.标的K线.序号）
                // - down 段被底分型终止：底分型中间 = 谷底 = 上行笔的文端（底）
                // - up 段被顶分型终止：顶分型中间 = 峰顶 = 下行笔的文端（顶）
                // 两种情况都取 wen 端
                let (wu_bar, wu_v) = (bis[mid_wen].wen, bis[mid_wen].wen_feat);
                segs.push(Segment {
                    dir_up,
                    wen: bis[i].wen,
                    wu: wu_bar,
                    wen_feat: bis[i].wen_feat,
                    wu_feat: wu_v,
                    start_bi: i,
                    end_bi,
                });
                // 四象传递：上段已是老阳/老阴链 → 新段前一缺口重置
                prev_gap = if prev_gap { false } else { gap };
                feat_from = i; // 特征分析输入 = 新段基础序列本身
                i = mid_wen;
            }
            _ => {
                // 尾段（未终结）：武=极值端
                let mut best = i;
                for k in i..bis.len() {
                    let (v, bv) = (bis[k].wu_feat, bis[best].wu_feat);
                    if (dir_up && v > bv) || (!dir_up && v < bv) {
                        best = k;
                    }
                }
                // 缺口突破修正（chan.py L5698-5754 的批处理等价）：
                // 末段方向与突破笔反向、末段处于老阳/老阴链（其起点特征间隔有缺口）、
                // 且反向笔突破前段终点方向的极值 → 撤销末段并入前段，前段延至突破极值处。
                // 例：zigzag (459,524)+(524,568) → bi32 低 117.34 无突破……实际突破=bi32? 
                // py 日志确认触发=缺口突破：段(524,568) 老阴、反向笔(591→599) 高 133.67>段高 133.52。
                let mut merged_ok = false;
                if segs.len() >= 2 && prev_gap {
                    // 前段处于老阳/老阴链：其产生时继承的终止间隔缺口 = prev_gap
                    let Some(prev) = segs.last() else { break };
                    // L5728：老阳(向上段)→反向笔低点破段低；老阴(向下段)→反向笔高点破段高
                    let prev_high = prev.wen_feat.max(prev.wu_feat);
                    let prev_low = prev.wen_feat.min(prev.wu_feat);
                    // 突破笔 = 未终结段内（i..=best）与末段方向反向、突破末段端点极值的笔
                    // （chan.py 逐笔处理时的"当前虚线"= 段基础序列的最新笔）
                    let breakthrough = (i..=best).any(|k| {
                        let l = &bis[k];
                        if prev.dir_up {
                            !l.dir_up && l.low < prev_low
                        } else {
                            l.dir_up && l.high > prev_high
                        }
                    });
                    if breakthrough {
                        // 重扫前段起点：合并后不得出现更晚的新终止（否则前段原终止有效，非突破）
                        let view_chk: Vec<usize> = (prev.start_bi..bis.len()).collect();
                        let new_term = scan_features(bis, &view_chk, prev.dir_up, false).terminated;
                        let prev_still_valid = match new_term {
                            Some((chk_end, _, _)) => chk_end > prev.end_bi && chk_end < bis.len() - 1,
                            None => false,
                        };
                        if !prev_still_valid {
                            // 前段基础序列 += 被撤段基础序列，重新刷新（L5742-5753）
                            let Some(popped) = segs.pop() else { break };
                            let new_start = popped.start_bi;
                            let new_dir = popped.dir_up;
                            // 合并后的段终点 = 段方向上的极值端（在突破笔之前）——
                            // py 语义：前段基础序列 += 被撤段基础序列，武随极值刷新（L5284 武斗）。
                            // 突破笔(反向) 不改变方向极值，故终点 = 到突破笔文端前的方向极值。
                            let mut bk = new_start;
                            for k in new_start..bis.len() {
                                let (v, bv) = (bis[k].wu_feat, bis[bk].wu_feat);
                                if (new_dir && v > bv) || (!new_dir && v < bv) {
                                    bk = k;
                                }
                            }
                            segs.push(Segment {
                                dir_up: new_dir,
                                wen: bis[new_start].wen,
                                wu: bis[bk].wu,
                                wen_feat: bis[new_start].wen_feat,
                                wu_feat: bis[bk].wu_feat,
                                start_bi: new_start,
                                end_bi: bis.len() - 1,
                            });
                            merged_ok = true;
                        }
                        // 原终止仍成立 → 不合并，按正常尾段处理（落到下方 !merged_ok 分支）
                    }
                }
                if !merged_ok {
                    segs.push(Segment {
                        dir_up,
                        wen: bis[i].wen,
                        wu: bis[best].wu,
                        wen_feat: bis[i].wen_feat,
                        wu_feat: bis[best].wu_feat,
                        start_bi: i,
                        end_bi: bis.len() - 1,
                    });
                }
                break;
            }
        }
    }
    segs
}

/// MACD（chan.py L1441 `增量计算` 精确口径）：
/// - EMA α=2/(N+1)，种子=round2(首收盘)；**链上传递 round2 后的前值**（存储即状态）
/// - 本轮 DIF/DEA/柱 用本轮 **raw EMA** 参与计算后输出时才 round2
///   （L1472 `DIF = 快线EMA - 慢线EMA` 在 round 之前；DEA 递推同样用 raw DIF）
/// - 柱 = DIF − DEA（L1481，×2 被注释）
pub struct MacdOut {
    pub dif: Vec<f64>,
    pub dea: Vec<f64>,
    pub hist: Vec<f64>,
}

fn round2(x: f64) -> f64 {
    // Python round(x, 2) 为 banker's rounding（ties-to-even）
    (x * 100.0).round_ties_even() / 100.0
}

pub fn macd(closes: &[f64], fast: usize, slow: usize, signal: usize) -> MacdOut {
    let af = 2.0 / (fast as f64 + 1.0);
    let as_ = 2.0 / (slow as f64 + 1.0);
    let ag = 2.0 / (signal as f64 + 1.0);
    let kf = (fast as f64 - 1.0) / (fast as f64 + 1.0);
    let ks = (slow as f64 - 1.0) / (slow as f64 + 1.0);
    let kg = (signal as f64 - 1.0) / (signal as f64 + 1.0);
    let n = closes.len();
    let mut dif = vec![0.0; n];
    let mut dea = vec![0.0; n];
    let mut hist = vec![0.0; n];
    if n == 0 {
        return MacdOut { dif, dea, hist };
    }
    // 链状态 = round2 后的存储值（与 Python 对象字段一致）
    let mut ef = round2(closes[0]);
    let mut es = round2(closes[0]);
    let mut de = 0.0f64; // DEA_EMA 种子 = DIF(首) = 0
    for i in 0..n {
        let (ef_raw, es_raw) = if i == 0 {
            (ef, es)
        } else {
            (
                closes[i] * af + ef * kf, // 递推读 round 后前值；本轮 raw
                closes[i] * as_ + es * ks,
            )
        };
        let d_raw = ef_raw - es_raw; // L1472：raw 相减
        let de_raw = if i == 0 { 0.0 } else { d_raw * ag + de * kg }; // DEA 本轮 raw 值
        dif[i] = round2(d_raw);
        dea[i] = round2(de_raw); // 存储输出与下一轮递推同值（L1478 计算用 raw，存储 round）
        hist[i] = round2(d_raw - de_raw); // L1481：raw − raw，之后才 round
        de = dea[i]; // 下一轮递推读 round 后存储值
        ef = round2(ef_raw);
        es = round2(es_raw);
    }
    MacdOut { dif, dea, hist }
}

/// 区间 MACD 柱面积（chan.py L3112 `K线.获取MACD`）
/// mode_total=true → "总"（阳+|阴|）；false → 按方向取"阳"(up)/"阴"(down)
pub fn macd_area(hist: &[f64], a: usize, b: usize, up_dir: bool, mode_total: bool) -> f64 {
    let mut yang = 0.0;
    let mut yin = 0.0;
    for i in a..=b {
        if hist[i] >= 0.0 {
            yang += hist[i];
        } else {
            yin += hist[i];
        }
    }
    if mode_total {
        yang + yin.abs()
    } else if up_dir {
        yang
    } else {
        yin
    }
}

/// 买卖点（chan.py 观察者.识别买卖点 为占位 L7296；按标准缠论规则实现，
/// 规则见 docs/semantics.md §7）
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mmd {
    Buy1,
    Sell1,
    Buy2,
    Sell2,
    Buy3,
    Sell3,
    PzBeichi,
}

#[derive(Clone, Debug)]
pub struct MmdMark {
    /// 出现的原始 bar 序号
    pub bar: usize,
    pub kind: Mmd,
}

/// 中枢进入段：start_li 的前一线
fn p_enter(p: &Pivot) -> Option<usize> {
    if p.start_li == 0 {
        None
    } else {
        Some(p.start_li - 1)
    }
}

/// 中枢离开段：优先第三买卖线，否则末线后第一笔
fn p_leave(lines: &[Line], p: &Pivot) -> Option<usize> {
    if let Some(t) = p.tbs {
        return Some(t);
    }
    if p.end_li + 1 < lines.len() {
        Some(p.end_li + 1)
    } else {
        None
    }
}

pub fn detect_mmds(lines: &[Line], bi_pivots: &[Pivot], hist: &[f64]) -> Vec<MmdMark> {
    let mut out: Vec<MmdMark> = Vec::new();
    // ---- 三买/三卖（L5473-5489 口径：离开笔与中枢呈缺口关系）----
    for p in bi_pivots {
        let mut cand: Option<usize> = None;
        for j in (p.end_li + 1)..lines.len() {
            let l = &lines[j];
            if rel(p.zg, p.zd, l.high, l.low).is_gap() {
                cand = Some(j);
                break;
            } else {
                break; // 首笔未离开即停
            }
        }
        if let Some(j) = cand {
            let l = &lines[j];
            if !l.dir_up && l.low > p.zg {
                out.push(MmdMark { bar: l.wu, kind: Mmd::Buy3 });
            }
            if l.dir_up && l.high < p.zd {
                out.push(MmdMark { bar: l.wu, kind: Mmd::Sell3 });
            }
        }
    }
    // ---- 一买/一卖：趋势（相邻两中枢同向且区间不重叠）+ 离开段 MACD 面积背驰 ----
    for w in bi_pivots.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let down_trend = b.zg < a.zd; // 下方中枢 → 向下趋势
        let up_trend = b.zd > a.zg; // 上方中枢 → 向上趋势
        if !down_trend && !up_trend {
            continue;
        }
        if let (Some(e), Some(v)) = (p_enter(b), p_leave(lines, b)) {
            let dir_up = lines[v].dir_up;
            let (ae, vl) = (
                macd_area(hist, lines[e].wen, lines[e].wu, dir_up, false),
                macd_area(hist, lines[v].wen, lines[v].wu, dir_up, false),
            );
            if down_trend && !dir_up {
                // 创新低 + 面积收缩 → 一买
                if lines[v].low < lines[b.start_li].low && ae != 0.0 && vl.abs() < ae.abs() {
                    out.push(MmdMark { bar: lines[v].wu, kind: Mmd::Buy1 });
                }
            }
            if up_trend && dir_up {
                if lines[v].high > lines[b.start_li].high && ae != 0.0 && vl.abs() < ae.abs() {
                    out.push(MmdMark { bar: lines[v].wu, kind: Mmd::Sell1 });
                }
            }
        }
    }
    // ---- 二买/二卖：一买/一卖（向下/向上笔终点）后次级回调不破极值 ----
    let one_bars: Vec<(usize, bool)> = out
        .iter()
        .filter_map(|m| match m.kind {
            Mmd::Buy1 => Some((m.bar, false)),
            Mmd::Sell1 => Some((m.bar, true)),
            _ => None,
        })
        .collect();
    for (bar, is_sell) in one_bars {
        let Some(bi_idx) = lines.iter().position(|l| l.wu == bar) else { continue };
        if bi_idx + 2 < lines.len() {
            let d = &lines[bi_idx + 2];
            if !is_sell && !d.dir_up && d.low > lines[bi_idx].low {
                out.push(MmdMark { bar: d.wu, kind: Mmd::Buy2 });
            }
            if is_sell && d.dir_up && d.high < lines[bi_idx].high {
                out.push(MmdMark { bar: d.wu, kind: Mmd::Sell2 });
            }
        }
    }
    // ---- 盘整背驰标记：中枢进入段 vs 离开段（创极值时）----
    for p in bi_pivots {
        if let (Some(e), Some(v)) = (p_enter(p), p_leave(lines, p)) {
            let dir_up = lines[v].dir_up;
            let ae = macd_area(hist, lines[e].wen, lines[e].wu, dir_up, false);
            let vl = macd_area(hist, lines[v].wen, lines[v].wu, dir_up, false);
            let new_ext = if dir_up {
                lines[v].high > lines[p.start_li].high
            } else {
                lines[v].low < lines[p.start_li].low
            };
            if new_ext && ae != 0.0 && vl.abs() < ae.abs() {
                out.push(MmdMark { bar: lines[v].wu, kind: Mmd::PzBeichi });
            }
        }
    }
    out.sort_by_key(|m| m.bar);
    out
}

pub fn dbg_scan(bis: &[Line]) -> (usize, bool, usize) {
    let view: Vec<usize> = (0..bis.len()).collect();
    let scan = scan_features(bis, &view, false, false);
    match scan.terminated {
        Some((end_bi, gap, _)) => (end_bi, gap, scan.feats.len()),
        None => (usize::MAX, false, scan.feats.len()),
    }
}

pub fn dbg_scan2(bis: &[Line]) -> Vec<(f64, f64)> {
    let view: Vec<usize> = (0..bis.len()).collect();
    let scan = scan_features(bis, &view, false, false);
    scan.feats.iter().map(|f| (f.high, f.low)).collect()
}

pub fn dbg_scan3(bis: &[Line], from: usize, dir_up: bool) -> (usize, usize) {
    let view: Vec<usize> = (from.saturating_sub(1)..bis.len()).collect();
    let scan = scan_features(bis, &view, dir_up, false);
    match scan.terminated {
        Some((e, _, _)) => (e, scan.feats.len()),
        None => (usize::MAX, scan.feats.len()),
    }
}

pub fn dbg_scan_view(bis: &[Line], view: &[usize], dir_up: bool) -> (Option<usize>, usize) {
    let scan = scan_features(bis, view, dir_up, false);
    (scan.terminated.map(|(e, _, _)| e), scan.feats.len())
}

pub fn dbg_feats(bis: &[Line], view: &[usize], dir_up: bool) -> Vec<(f64, f64)> {
    let scan = scan_features(bis, view, dir_up, false);
    scan.feats.iter().map(|f| (f.high, f.low)).collect()
}
