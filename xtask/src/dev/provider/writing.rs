//! What the dev provider's replies say: invented but ordinary engineering
//! answers, chosen to fit the prompt where one fits, so the slot's history
//! reads like real use. Nothing here is anyone's real work.

use serde_json::{Value, json};

/// The text and tool calls of replies.
pub struct Writing;

/// A reply's opening, matched by a word in the prompt.
const ANSWERS: [(&str, &str); 8] = [
    (
        "CI runner",
        "The CI image pins an older OpenSSL than your machine, and the release profile enables the `native-tls` feature, so the build links against headers that are not there. Locally the dev profile uses `rustls` and never touches it. I switched the release profile to `rustls` as well and removed the unused feature from `acme-web/Cargo.toml`; the release build now passes in the same container the runner uses.",
    ),
    (
        "retry",
        "I added a `Backoff` type next to the upload client: three attempts, starting at 200 ms and doubling, with jitter, and only for connection errors and 502/503/504. A 4xx is returned at once, since retrying it cannot help. The tests use a test transport that fails twice and then succeeds, and another that always returns 400 to check that it is not retried.",
    ),
    (
        "listen_addr",
        "There were eleven uses across four services and the shared config crate. I renamed the field to `listen`, kept `listen_addr` as a deprecated alias in the deserializer so existing config files still load with a warning, and updated the two example configs and the deployment docs. `cargo check --workspace` and the config tests pass.",
    ),
    (
        "job queue",
        "When a worker stops responding, its lease on the job runs out after 30 seconds and the job goes back on the queue with its attempt count increased. After five attempts it moves to the dead-letter table instead, so one bad job cannot block the others. Nothing is lost if the worker comes back late: its result is ignored because the lease token no longer matches.",
    ),
    (
        "migration",
        "The migration adds `CREATE INDEX CONCURRENTLY idx_orders_created_at ON orders (created_at)` so it does not lock the table while it builds, and the down migration drops it. I checked the two slow queries from the dashboard: both now use the index, and the order list endpoint goes from about 900 ms to 40 ms on the staging data.",
    ),
    (
        "review comments",
        "There are six open threads on the checkout PR. Two are about naming and are easy to take. Three ask for tests around the discount rounding, which is fair, since that path had none. The last one questions whether the cart should be validated on the server again before payment; I agree it should, and it is a small change.",
    ),
    (
        "depend on",
        "It only needs one thing from the inventory crate: the `StockLevel` type, to refuse an invoice for items that are out of stock. That check belongs in the order service, which already knows both. Moving it there removes the dependency entirely, and billing stops rebuilding every time inventory changes.",
    ),
    (
        "sorted",
        "The list was sorted in the handler with `sort_by_key(|order| order.created_at)`, which is oldest first. I moved the ordering into the query as `ORDER BY created_at DESC, id DESC`, so pagination stays stable, and removed the in-memory sort. The list test now checks the order explicitly.",
    ),
];

/// A reply for a prompt that matches none of the above.
const FALLBACK: [&str; 3] = [
    "I read through the relevant files first, then traced where the value is set and where it is used. The change is small: one function moves, two call sites follow it, and the tests that cover it still pass. I left a short note in the code where the behaviour is easy to miss.",
    "The behaviour comes from the default in the config loader, not from the service itself: when the field is missing it falls back to the old value. I made the default explicit, added a test for the missing-field case, and the existing tests still pass.",
    "Short answer: yes, but only on the write path. Reads go through the cache and never reach it. I added a test that covers the write path and documented the reason next to the call, since it is not obvious from the code.",
];

/// Paragraphs a long answer is made of.
const DETAIL: [&str; 6] = [
    "First, the shape of the problem. The two crates were split early, when billing and inventory lived in one service, and the dependency was never revisited once they moved apart. Today billing imports inventory for a single type, but because the crates are in one workspace, every change to inventory rebuilds billing and reruns its tests.",
    "Second, what actually crosses the boundary. Billing reads `StockLevel` in one place: before it issues an invoice it checks that every line item is in stock. That is an ordering rule, not a billing rule, and the order service already loads both the cart and the stock levels before it asks billing for an invoice.",
    "Third, what breaks if we cut the edge. Nothing at runtime: the check moves earlier, into the order service, and billing trusts the order it is given. Two tests in billing that build a stub stock level have to move with the check. The public API of both crates stays the same.",
    "Fourth, the migration. I would do it in two changes: add the check to the order service behind the existing feature flag and run both for a week, comparing results in the logs; then remove the check and the dependency from billing. Each change is small enough to review on its own and to revert.",
    "Fifth, what we gain. Billing builds in about half the time, its tests no longer depend on inventory fixtures, and the next person reading the invoice code does not have to understand stock levels to change it.",
    "Finally, a note on risk. The only behaviour change is when an out-of-stock order is refused: before the invoice instead of during it. The customer sees the same message, a little earlier. I checked the support logs for the last month and found no flow that depends on the later refusal.",
];

/// Tool calls a reply can make, with what it says before them.
const TOOLS: [(&str, &str, &str, &str); 5] = [
    (
        "Let me look at how it is built first.",
        "Bash",
        "command",
        "cargo build --release -p acme-web 2>&1 | tail -20",
    ),
    (
        "I'll check where it is used.",
        "Bash",
        "command",
        "rg -n \"listen_addr\" --type rust",
    ),
    (
        "Let me read the client before changing it.",
        "Read",
        "file_path",
        "src/upload/client.rs",
    ),
    (
        "I'll run the tests to see what fails.",
        "Bash",
        "command",
        "cargo test -p acme-billing",
    ),
    (
        "Let me check the recent history of this file.",
        "Bash",
        "command",
        "git log --oneline -10 -- src/orders/list.rs",
    ),
];

impl Writing {
    /// The answer to `prompt`: a fitting one when a word matches, else one of
    /// a few general ones, picked by `seed`.
    pub fn answer(prompt: &str, seed: u64) -> &'static str {
        ANSWERS
            .iter()
            .find(|(word, _)| prompt.contains(word))
            .map(|(_, text)| *text)
            .unwrap_or_else(|| FALLBACK[Self::index(seed, FALLBACK.len())])
    }

    /// A long, structured answer: every paragraph, after a fitting opening.
    pub fn long_answer(prompt: &str, seed: u64) -> String {
        let mut text = format!("{}\n\n", Self::answer(prompt, seed));
        for paragraph in DETAIL {
            text.push_str(paragraph);
            text.push_str("\n\n");
        }
        text
    }

    /// What a reply says before a tool call, the tool, and its input.
    pub fn tool(seed: u64) -> (&'static str, &'static str, Value) {
        let (lead, name, field, value) = TOOLS[Self::index(seed, TOOLS.len())];
        let mut input = json!({ field: value });
        if name == "Bash" {
            input["description"] = json!("Run in the project root");
        }
        (lead, name, input)
    }

    fn index(seed: u64, len: usize) -> usize {
        usize::try_from(seed % u64::try_from(len).unwrap_or(1)).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::Writing;

    #[test]
    fn answers_fit_the_prompt_and_never_say_they_are_fake() {
        assert!(
            Writing::answer(
                "Why does the release build of acme-web fail on the CI runner?",
                0
            )
            .contains("OpenSSL")
        );
        assert!(!Writing::answer("Something else entirely", 4).is_empty());
        let long = Writing::long_answer("Why should acme-billing depend on acme-inventory?", 0);
        assert!(long.len() > 2_000, "{}", long.len());
        let every = super::ANSWERS
            .iter()
            .map(|(_, text)| *text)
            .chain(super::FALLBACK);
        for text in every.chain([long.as_str(), Writing::tool(2).0]) {
            assert!(!text.to_lowercase().contains("fake"), "{text}");
        }
    }
}
