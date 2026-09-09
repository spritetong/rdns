//! Notification event definitions.

#[derive(Debug, Clone)]
pub enum NotificationEvent<'a> {
    Change {
        task_name: &'a str,
        old_ip: &'a str,
        new_ip: &'a str,
    },
    Failure {
        task_name: &'a str,
        error_message: &'a str,
    },
    Recovery {
        task_name: &'a str,
        current_ip: &'a str,
    },
}
