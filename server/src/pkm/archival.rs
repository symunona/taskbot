use crate::pkm::threads::ThreadManager;

pub struct ArchivalManager {
    thread_manager: ThreadManager,
}

impl ArchivalManager {
    pub fn new(thread_manager: ThreadManager) -> Self {
        Self { thread_manager }
    }

    pub fn archive_thread(&self, thread_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.thread_manager.close_thread(thread_id)
    }

    pub fn serialize_conversation(
        &self,
        user_text: &str,
        model_text: &str,
        system_blocks: &str,
    ) -> String {
        let mut serialized = String::new();

        if !user_text.is_empty() {
            serialized.push_str(&format!("**User**:\n{}\n\n", user_text));
        }

        if !model_text.is_empty() {
            serialized.push_str(&format!(
                "**Model**:\n> {}\n\n",
                model_text.replace("\n", "\n> ")
            ));
        }

        if !system_blocks.is_empty() {
            serialized.push_str(&format!("**System**:\n```\n{}\n```\n\n", system_blocks));
        }

        serialized
    }

    // Placeholder for Gemini integration to summarize and auto-name threads
    pub fn summarize_thread(&self, _thread_id: &str) -> Result<String, Box<dyn std::error::Error>> {
        // TODO: Integrate with Gemini API to generate summary
        Ok("Generated summary placeholder".to_string())
    }

    pub fn auto_name_thread(&self, _thread_id: &str) -> Result<String, Box<dyn std::error::Error>> {
        // TODO: Integrate with Gemini API to generate topic-keywords filename
        Ok("YYYY-MM-DD-topic-keywords".to_string())
    }
}
