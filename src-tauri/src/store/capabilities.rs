use anyhow::Result;

use crate::models::{
    Agent, ChannelSessionInfo, KanbanItem, KanbanStatus, ProjectInfo, SessionInfo,
};

use super::SessionStore;

pub trait ProjectStore {
    fn list_projects(&self) -> Result<Vec<ProjectInfo>>;
    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo>;
    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        process_template_id: Option<String>,
    ) -> Result<ProjectInfo>;
    fn archive_project(&self, project_id: &str) -> Result<()>;
}

impl<T: SessionStore + ?Sized> ProjectStore for T {
    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        SessionStore::list_projects(self)
    }

    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        SessionStore::add_project(self, path, name, process_template_id, enabled_stage_ids)
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        process_template_id: Option<String>,
    ) -> Result<ProjectInfo> {
        SessionStore::update_project(self, project_id, name, process_template_id)
    }

    fn archive_project(&self, project_id: &str) -> Result<()> {
        SessionStore::archive_project(self, project_id)
    }
}

pub trait KanbanStore {
    fn list_kanban_items(&self, project_id: &str) -> Result<Vec<KanbanItem>>;
    fn create_kanban_item(
        &self,
        project_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<KanbanItem>;
    fn update_kanban_item(
        &self,
        item_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<KanbanStatus>,
    ) -> Result<KanbanItem>;
    fn delete_kanban_item(&self, item_id: &str) -> Result<()>;
}

impl<T: SessionStore + ?Sized> KanbanStore for T {
    fn list_kanban_items(&self, project_id: &str) -> Result<Vec<KanbanItem>> {
        SessionStore::list_kanban_items(self, project_id)
    }

    fn create_kanban_item(
        &self,
        project_id: &str,
        title: &str,
        description: Option<&str>,
    ) -> Result<KanbanItem> {
        SessionStore::create_kanban_item(self, project_id, title, description)
    }

    fn update_kanban_item(
        &self,
        item_id: &str,
        title: Option<&str>,
        description: Option<Option<&str>>,
        status: Option<KanbanStatus>,
    ) -> Result<KanbanItem> {
        SessionStore::update_kanban_item(self, item_id, title, description, status)
    }

    fn delete_kanban_item(&self, item_id: &str) -> Result<()> {
        SessionStore::delete_kanban_item(self, item_id)
    }
}

pub trait SessionCommandStore {
    fn list_sessions(&self) -> Result<Vec<SessionInfo>>;
    fn list_channel_sessions(&self) -> Result<Vec<ChannelSessionInfo>>;
    fn update_session_rename_title(
        &self,
        agent: Agent,
        session_id: &str,
        rename_title: Option<&str>,
    ) -> Result<()>;
}

impl<T: SessionStore + ?Sized> SessionCommandStore for T {
    fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        SessionStore::list_sessions(self)
    }

    fn list_channel_sessions(&self) -> Result<Vec<ChannelSessionInfo>> {
        SessionStore::list_channel_sessions(self)
    }

    fn update_session_rename_title(
        &self,
        agent: Agent,
        session_id: &str,
        rename_title: Option<&str>,
    ) -> Result<()> {
        SessionStore::update_session_rename_title(self, agent, session_id, rename_title)
    }
}
