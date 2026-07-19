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
    pub(crate) session: Session<Screencast>,
}

pub struct PortalCapture;

struct SessionGrantParts<S, R> {
    stream: S,
    remote: R,
    restore_token: Option<String>,
}

#[async_trait::async_trait]
trait PortalSessionOperations: Send {
    type Stream: Send;
    type Remote: Send;

    async fn select_sources(&mut self) -> Result<(), CaptureError>;
    async fn start(&mut self) -> Result<(Vec<Self::Stream>, Option<String>), CaptureError>;
    async fn open_remote(&mut self) -> Result<Self::Remote, CaptureError>;
    async fn close_session(&mut self) -> Result<(), CaptureError>;
}

struct AshpdSessionOperations<'a> {
    portal: &'a Screencast,
    session: &'a Session<Screencast>,
    restore_token: Option<String>,
}

#[async_trait::async_trait]
impl PortalSessionOperations for AshpdSessionOperations<'_> {
    type Stream = Stream;
    type Remote = OwnedFd;

    async fn select_sources(&mut self) -> Result<(), CaptureError> {
        self.portal
            .select_sources(
                self.session,
                selection_options(self.restore_token.take()).into_portal_options(),
            )
            .await
            .map_err(portal_error)?
            .response()
            .map_err(portal_error)
    }

    async fn start(&mut self) -> Result<(Vec<Self::Stream>, Option<String>), CaptureError> {
        let response = self
            .portal
            .start(self.session, None, Default::default())
            .await
            .map_err(portal_error)?
            .response()
            .map_err(portal_error)?;
        Ok((
            response.streams().to_vec(),
            response.restore_token().map(ToOwned::to_owned),
        ))
    }

    async fn open_remote(&mut self) -> Result<Self::Remote, CaptureError> {
        self.portal
            .open_pipe_wire_remote(self.session, Default::default())
            .await
            .map_err(portal_error)
    }

    async fn close_session(&mut self) -> Result<(), CaptureError> {
        self.session.close().await.map_err(portal_error)
    }
}

impl PortalGrant {
    pub(crate) async fn close(&self) -> Result<(), CaptureError> {
        self.session.close().await.map_err(portal_error)
    }
}

impl PortalCapture {
    pub async fn open(restore_token: Option<String>) -> Result<PortalGrant, CaptureError> {
        let portal = Screencast::new().await.map_err(portal_error)?;
        let session = portal
            .create_session(Default::default())
            .await
            .map_err(portal_error)?;
        let mut operations = AshpdSessionOperations {
            portal: &portal,
            session: &session,
            restore_token,
        };
        let parts = complete_portal_session(&mut operations).await?;

        Ok(PortalGrant {
            stream: parts.stream,
            remote_fd: parts.remote,
            restore_token: parts.restore_token,
            session,
        })
    }
}

async fn complete_portal_session<O>(
    operations: &mut O,
) -> Result<SessionGrantParts<O::Stream, O::Remote>, CaptureError>
where
    O: PortalSessionOperations,
{
    let result = async {
        operations.select_sources().await?;
        let (streams, restore_token) = operations.start().await?;
        if streams.len() != 1 {
            return Err(CaptureError::UnexpectedStreamCount {
                count: streams.len(),
            });
        }
        let stream = streams
            .into_iter()
            .next()
            .expect("exactly one stream was checked");
        let remote = operations.open_remote().await?;
        Ok(SessionGrantParts {
            stream,
            remote,
            restore_token,
        })
    }
    .await;

    match result {
        Ok(parts) => Ok(parts),
        Err(primary) => match operations.close_session().await {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(CaptureError::Cleanup {
                primary: Box::new(primary),
                cleanup: Box::new(cleanup),
            }),
        },
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

    #[derive(Clone, Copy, Debug)]
    enum FailureStage {
        Select,
        Start,
        StreamCount,
        Remote,
    }

    struct FakePortalSession {
        failure: FailureStage,
        closes: usize,
    }

    #[async_trait::async_trait]
    impl super::PortalSessionOperations for FakePortalSession {
        type Remote = ();
        type Stream = u8;

        async fn select_sources(&mut self) -> Result<(), CaptureError> {
            if matches!(self.failure, FailureStage::Select) {
                Err(fake_portal_error("select"))
            } else {
                Ok(())
            }
        }

        async fn start(&mut self) -> Result<(Vec<Self::Stream>, Option<String>), CaptureError> {
            if matches!(self.failure, FailureStage::Start) {
                return Err(fake_portal_error("start"));
            }
            let streams = if matches!(self.failure, FailureStage::StreamCount) {
                vec![1, 2]
            } else {
                vec![1]
            };
            Ok((streams, Some("new-token".to_owned())))
        }

        async fn open_remote(&mut self) -> Result<Self::Remote, CaptureError> {
            if matches!(self.failure, FailureStage::Remote) {
                Err(fake_portal_error("remote"))
            } else {
                Ok(())
            }
        }

        async fn close_session(&mut self) -> Result<(), CaptureError> {
            self.closes += 1;
            Ok(())
        }
    }

    fn fake_portal_error(stage: &str) -> CaptureError {
        CaptureError::Portal {
            message: format!("injected {stage} failure"),
        }
    }

    #[tokio::test]
    async fn every_post_creation_portal_failure_closes_the_session() {
        for failure in [
            FailureStage::Select,
            FailureStage::Start,
            FailureStage::StreamCount,
            FailureStage::Remote,
        ] {
            let mut session = FakePortalSession { failure, closes: 0 };

            let result = super::complete_portal_session(&mut session).await;

            assert!(result.is_err(), "{failure:?} should fail");
            assert_eq!(session.closes, 1, "{failure:?} should close once");
        }
    }
}
