use std::os::fd::OwnedFd;

use ashpd::desktop::{
    PersistMode, ResponseError, Session,
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream},
};

use super::CaptureError;

pub struct PortalGrant {
    pub stream: Stream,
    pub remote_fd: OwnedFd,
    pub restore_token: Option<String>,
    pub(crate) _session: Session<Screencast>,
}

pub struct PortalCapture;

impl PortalCapture {
    pub async fn open(restore_token: Option<String>) -> Result<PortalGrant, CaptureError> {
        let portal = Screencast::new().await.map_err(portal_error)?;
        let session = portal
            .create_session(Default::default())
            .await
            .map_err(portal_error)?;

        portal
            .select_sources(
                &session,
                selection_options(restore_token).into_portal_options(),
            )
            .await
            .map_err(portal_error)?
            .response()
            .map_err(portal_error)?;

        let response = portal
            .start(&session, None, Default::default())
            .await
            .map_err(portal_error)?
            .response()
            .map_err(portal_error)?;

        if response.streams().len() != 1 {
            return Err(CaptureError::UnexpectedStreamCount {
                count: response.streams().len(),
            });
        }
        let stream = response.streams()[0].clone();
        let restore_token = response.restore_token().map(ToOwned::to_owned);
        let remote_fd = portal
            .open_pipe_wire_remote(&session, Default::default())
            .await
            .map_err(portal_error)?;

        Ok(PortalGrant {
            stream,
            remote_fd,
            restore_token,
            _session: session,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
struct CaptureSelectionOptions {
    source: SourceType,
    multiple: bool,
    cursor: CursorMode,
    persist: PersistMode,
    restore_token: Option<String>,
}

impl CaptureSelectionOptions {
    fn into_portal_options(self) -> SelectSourcesOptions {
        SelectSourcesOptions::default()
            .set_sources(Some(self.source.into()))
            .set_multiple(self.multiple)
            .set_cursor_mode(self.cursor)
            .set_persist_mode(self.persist)
            .set_restore_token(self.restore_token.as_deref())
    }
}

fn selection_options(restore_token: Option<String>) -> CaptureSelectionOptions {
    CaptureSelectionOptions {
        source: SourceType::Monitor,
        multiple: false,
        cursor: CursorMode::Hidden,
        persist: PersistMode::ExplicitlyRevoked,
        restore_token,
    }
}

fn portal_error(error: ashpd::Error) -> CaptureError {
    if matches!(error, ashpd::Error::Response(ResponseError::Cancelled)) {
        CaptureError::CaptureCancelled
    } else {
        CaptureError::Portal {
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use ashpd::desktop::{
        PersistMode, ResponseError,
        screencast::{CursorMode, SourceType},
    };

    use super::{portal_error, selection_options};
    use crate::capture::CaptureError;

    #[test]
    fn selection_is_one_hidden_cursor_monitor_with_persistent_permission() {
        let options = selection_options(Some("saved-token".to_owned()));

        assert_eq!(options.source, SourceType::Monitor);
        assert!(!options.multiple);
        assert_eq!(options.cursor, CursorMode::Hidden);
        assert_eq!(options.persist, PersistMode::ExplicitlyRevoked);
        assert_eq!(options.restore_token.as_deref(), Some("saved-token"));
    }

    #[test]
    fn portal_cancellation_is_a_typed_user_state() {
        let error = portal_error(ResponseError::Cancelled.into());

        assert!(matches!(error, CaptureError::CaptureCancelled));
    }
}
