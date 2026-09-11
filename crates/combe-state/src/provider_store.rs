use crate::{
    ProviderProfile, ProviderProfileId, Recipient, RecipientId, Result, RoutedProviderResult,
    WorkId, WorkMessage,
};

pub trait ProviderRegistry: Send + Sync {
    fn list_profiles(&self) -> Result<Vec<ProviderProfile>>;
    fn get_profile(&self, id: &ProviderProfileId) -> Result<ProviderProfile>;
    fn create_profile(&self, profile: ProviderProfile) -> Result<()>;
    fn update_profile(&self, profile: ProviderProfile) -> Result<()>;
    fn delete_profile(&self, id: &ProviderProfileId) -> Result<()>;
    fn list_recipients(&self) -> Result<Vec<Recipient>>;
    fn get_recipient(&self, id: &RecipientId) -> Result<Recipient>;
    fn create_recipient(&self, recipient: Recipient) -> Result<()>;
    fn update_recipient(&self, recipient: Recipient) -> Result<()>;
    fn delete_recipient(&self, id: &RecipientId) -> Result<()>;
    fn add_message(&self, message: WorkMessage) -> Result<()>;
    fn messages(&self, work_id: &WorkId) -> Result<Vec<WorkMessage>>;
    fn add_provider_result(&self, result: RoutedProviderResult) -> Result<()>;
    fn provider_results(&self, work_id: &WorkId) -> Result<Vec<RoutedProviderResult>>;
}
