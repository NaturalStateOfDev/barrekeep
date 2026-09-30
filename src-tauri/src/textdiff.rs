//! Small line-based unified diff (LCS), used to show a code draft against
//! the active algorithm script before Adopt. In-repo instead of a crate:
//! scripts are ~1k lines, so an O(n·m) table on the trimmed middle is
//! plenty, and it keeps the dependency list short (CLAUDE.md).

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Equal,
    Delete,
    Insert,
}

/// Line ops turning `a` into `b`: (op, a index, b index) where the index for
/// the side the op doesn't touch is the position it applies at.
fn line_ops<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<(Op, usize, usize)> {
    // Trim the common prefix/suffix so the DP only covers the changed middle.
    let mut pre = 0;
    while pre < a.len() && pre < b.len() && a[pre] == b[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < a.len() - pre && suf < b.len() - pre && a[a.len() - 1 - suf] == b[b.len() - 1 - suf]
    {
        suf += 1;
    }
    let am = &a[pre..a.len() - suf];
    let bm = &b[pre..b.len() - suf];
    let (n, m) = (am.len(), bm.len());

    // lcs[i][j] = LCS length of am[i..] and bm[j..].
    let w = m + 1;
    let mut lcs = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * w + j] = if am[i] == bm[j] {
                lcs[(i + 1) * w + j + 1] + 1
            } else {
                lcs[(i + 1) * w + j].max(lcs[i * w + j + 1])
            };
        }
    }

    let mut ops = Vec::with_capacity(a.len() + b.len());
    for k in 0..pre {
        ops.push((Op::Equal, k, k));
    }
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && am[i] == bm[j] {
            ops.push((Op::Equal, pre + i, pre + j));
            i += 1;
            j += 1;
        } else if i < n && (j == m || lcs[(i + 1) * w + j] >= lcs[i * w + j + 1]) {
            // Deletions first on ties, so a replaced line reads -old / +new.
            ops.push((Op::Delete, pre + i, pre + j));
            i += 1;
        } else {
            ops.push((Op::Insert, pre + i, pre + j));
            j += 1;
        }
    }
    for k in 0..suf {
        ops.push((Op::Equal, a.len() - suf + k, b.len() - suf + k));
    }
    ops
}

/// Unified diff (`---`/`+++` headers, `@@` hunks, `context` lines around
/// each change). Empty string when the texts are identical line-for-line.
pub fn unified_diff(old: &str, new: &str, old_name: &str, new_name: &str, context: usize) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let ops = line_ops(&a, &b);
    let changes: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, (op, _, _))| *op != Op::Equal)
        .map(|(k, _)| k)
        .collect();
    if changes.is_empty() {
        return String::new();
    }

    // Group changes whose gap of equal lines is small enough to share a hunk.
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for &k in &changes {
        match groups.last_mut() {
            Some((_, end)) if k - *end <= 2 * context + 1 => *end = k,
            _ => groups.push((k, k)),
        }
    }

    let mut out = format!("--- {old_name}\n+++ {new_name}\n");
    for (first, last) in groups {
        let start = first.saturating_sub(context);
        let end = (last + context + 1).min(ops.len());
        let slice = &ops[start..end];
        let old_len = slice.iter().filter(|(op, _, _)| *op != Op::Insert).count();
        let new_len = slice.iter().filter(|(op, _, _)| *op != Op::Delete).count();
        let (_, a0, b0) = slice[0];
        let old_start = if old_len == 0 { a0 } else { a0 + 1 };
        let new_start = if new_len == 0 { b0 } else { b0 + 1 };
        out.push_str(&format!("@@ -{old_start},{old_len} +{new_start},{new_len} @@\n"));
        for &(op, ai, bi) in slice {
            match op {
                Op::Equal => out.push_str(&format!(" {}\n", a[ai])),
                Op::Delete => out.push_str(&format!("-{}\n", a[ai])),
                Op::Insert => out.push_str(&format!("+{}\n", b[bi])),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_empty() {
        assert_eq!(unified_diff("a\nb\n", "a\nb\n", "x", "y", 3), "");
    }

    #[test]
    fn single_change_with_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nFIVE\n6\n7\n8\n9\n";
        let d = unified_diff(old, new, "old", "new", 2);
        assert_eq!(
            d,
            "--- old\n+++ new\n@@ -3,5 +3,5 @@\n 3\n 4\n-5\n+FIVE\n 6\n 7\n"
        );
    }

    #[test]
    fn insert_delete_and_separate_hunks() {
        let old: String = (1..=30).map(|n| format!("{n}\n")).collect();
        let new: String = (1..=30)
            .filter(|n| *n != 27)
            .map(|n| if n == 3 { "3\nnew-after-3\n".to_string() } else { format!("{n}\n") })
            .collect();
        let d = unified_diff(&old, &new, "o", "n", 1);
        let hunks = d.matches("@@ -").count();
        assert_eq!(hunks, 2, "{d}");
        assert!(d.contains("+new-after-3\n"));
        assert!(d.contains("-27\n"));
        assert!(d.contains("@@ -3,2 +3,3 @@"), "{d}");
    }

    #[test]
    fn pure_insertion_into_empty() {
        let d = unified_diff("", "a\nb\n", "o", "n", 3);
        assert_eq!(d, "--- o\n+++ n\n@@ -0,0 +1,2 @@\n+a\n+b\n");
    }
}
