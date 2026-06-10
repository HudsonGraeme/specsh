use std::collections::HashMap;

pub struct Candidate {
    pub cmd: String,
    pub count: u32,
    pub source: &'static str,
}

pub struct Model {
    trans: HashMap<String, HashMap<String, u32>>,
    freq: HashMap<String, u32>,
}

impl Model {
    pub fn build(cmds: &[String]) -> Model {
        let clean: Vec<&String> = cmds
            .iter()
            .filter(|c| !c.contains('\n') && !c.trim().is_empty())
            .collect();
        let mut trans: HashMap<String, HashMap<String, u32>> = HashMap::new();
        let mut freq: HashMap<String, u32> = HashMap::new();
        for w in clean.windows(2) {
            *trans
                .entry(w[0].clone())
                .or_default()
                .entry(w[1].clone())
                .or_default() += 1;
        }
        for c in &clean {
            *freq.entry((*c).clone()).or_default() += 1;
        }
        Model { trans, freq }
    }

    pub fn candidates(&self, after: &str, k: usize) -> Vec<Candidate> {
        let mut out: Vec<Candidate> = Vec::new();
        if let Some(m) = self.trans.get(after) {
            let mut v: Vec<(&String, &u32)> = m.iter().collect();
            v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (c, n) in v.into_iter().take(k) {
                if *n >= 2 {
                    out.push(Candidate {
                        cmd: c.clone(),
                        count: *n,
                        source: "follows",
                    });
                }
            }
        }
        let mut f: Vec<(&String, &u32)> = self.freq.iter().collect();
        f.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (c, n) in f {
            if out.len() >= k * 2 {
                break;
            }
            if *n >= 3 && !out.iter().any(|x| x.cmd == *c) {
                out.push(Candidate {
                    cmd: c.clone(),
                    count: *n,
                    source: "frequent",
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn follows_beats_frequent() {
        let hist = v(&[
            "vim src/main.rs",
            "cargo test",
            "vim src/main.rs",
            "cargo test",
            "vim src/main.rs",
            "cargo test",
            "git status",
            "git status",
            "git status",
            "git status",
        ]);
        let m = Model::build(&hist);
        let c = m.candidates("vim src/main.rs", 3);
        assert_eq!(c[0].cmd, "cargo test");
        assert_eq!(c[0].source, "follows");
        assert_eq!(c[0].count, 3);
    }

    #[test]
    fn unknown_context_falls_back_to_frequent() {
        let hist = v(&["cargo test", "cargo test", "cargo test", "ls"]);
        let m = Model::build(&hist);
        let c = m.candidates("never seen this", 3);
        assert!(!c.is_empty());
        assert_eq!(c[0].cmd, "cargo test");
        assert_eq!(c[0].source, "frequent");
    }

    #[test]
    fn single_occurrence_transitions_ignored() {
        let hist = v(&["a", "b", "c", "d"]);
        let m = Model::build(&hist);
        assert!(m.candidates("a", 3).iter().all(|c| c.source != "follows"));
    }
}
