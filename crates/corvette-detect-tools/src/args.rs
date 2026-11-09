use crate::Failure;

#[derive(Debug)]
pub(crate) struct Flags {
    pairs: Vec<(String, String)>,
}

impl Flags {
    pub(crate) fn parse(args: &[String], known: &[&str]) -> Option<Self> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        let mut rest = args;
        while let Some((head, tail)) = rest.split_first() {
            let name = head.strip_prefix("--")?;
            if !known.contains(&name) {
                return None;
            }
            let (value, after) = tail.split_first()?;
            if pairs.iter().any(|(seen, _)| seen == name) {
                return None;
            }
            pairs.push((name.to_string(), value.clone()));
            rest = after;
        }
        Some(Self { pairs })
    }

    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(seen, _)| seen == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn required(&self, name: &str) -> Result<&str, Failure> {
        self.get(name).ok_or(Failure::Usage)
    }
}
