use std::collections::BTreeMap;

fn main() {
    let sessions = app_lib::agents::sources::list_all();
    println!("Total sessions: {}", sessions.len());

    let mut by_agent: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &sessions {
        *by_agent.entry(s.agent.as_str()).or_default() += 1;
    }
    for (agent, count) in &by_agent {
        println!("  {agent}: {count}");
    }

    println!("\nLatest 5 per agent:");
    for agent in ["codex", "claude", "gemini"] {
        let mut rows: Vec<_> = sessions
            .iter()
            .filter(|s| s.agent.as_str() == agent)
            .collect();
        rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        println!("\n[{agent}]");
        for s in rows.iter().take(5) {
            println!(
                "  - {} | id={} | msgs={}{} | size={}B",
                s.project_name.as_deref().unwrap_or("?"),
                &s.id[..s.id.len().min(12)],
                if s.partial { "~" } else { "" },
                s.message_count,
                s.file_size
            );
            if let Some(preview) = &s.first_user_message {
                println!("    “{}”", preview);
            }
        }
    }
}
