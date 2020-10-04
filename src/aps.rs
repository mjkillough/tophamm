use std::collections::HashMap;

use tokio::stream::Stream;
use tokio::sync::{mpsc, oneshot, watch};

use crate::protocol::RequestId;
use crate::{
    ApsDataConfirm, ApsDataIndication, ApsDataRequest, Deconz, DeviceState, ErrorKind, Request,
    Response, Result,
};

/// A command from Deconz to the Aps task, representing an ApsDataRequest.
pub struct ApsCommand {
    pub request: ApsDataRequest,
    pub sender: oneshot::Sender<Result<ApsDataConfirm>>,
}

/// Task responsible for handlign all APS requests.
///
/// Listens to device state to decide when to:
///
///  - Forward ApsDataRequest to the adapter.
///  - Request ApsDataIndications from the adapter, fowarding them to the ApsReader for the
///    application to process.
///  - Request ApsDataConfirms from the adapter, forwarding them to the future awaiting successful
///    confirmation of an ApsDataRequest.
pub struct Aps {
    pub deconz: Deconz,
    pub request_id: RequestId,
    pub request_free_slots: bool,
    pub device_state: watch::Receiver<DeviceState>,
    pub aps_data_requests: mpsc::Receiver<ApsCommand>,
    pub aps_data_indications: mpsc::Sender<ApsDataIndication>,
    pub awaiting: HashMap<RequestId, oneshot::Sender<Result<ApsDataConfirm>>>,
}

impl Aps {
    pub async fn task(mut self) -> Result<()> {
        loop {
            tokio::select! {
                Some(device_state) = self.device_state.recv() => {
                    debug!("aps: {:?}", device_state);

                    self.request_free_slots = device_state.data_request_free_slots;

                    if device_state.data_indication {
                        if let Err(error) = self.aps_data_indication().await {
                            error!("aps_data_indication: {:?}", error);
                        }
                    }

                    if device_state.data_confirm {
                        if let Err(error) = self.aps_data_confirm().await {
                            error!("aps_data_confirm: {:?}", error);
                        }
                    }
                }
                Some(ApsCommand { request, sender }) = self.aps_data_requests.recv(),
                    if self.request_free_slots =>
                {
                    // Assume we can only send one message. We'll get a DeviceState in the response
                    // which will tell us if we can send more.
                    self.request_free_slots = false;

                    match self.aps_data_request(request).await {
                        Ok(request_id) => {
                            self.awaiting.insert(request_id, sender);
                        },
                        Err(error) => {
                            error!("aps_data_request: {:?}", error);
                            let _ = sender.send(Err(error));
                        }
                    }

                }
                else => break,
            }
        }

        Ok(())
    }

    async fn aps_data_indication(&mut self) -> Result<()> {
        let response = self.deconz.make_request(Request::ApsDataIndication).await?;
        let aps_data_indication = match response {
            Response::ApsDataIndication {
                aps_data_indication,
                ..
            } => aps_data_indication,
            resp => return Err(ErrorKind::UnexpectedResponse(resp.command_id()).into()),
        };

        self.aps_data_indications
            .send(aps_data_indication)
            .await
            .map_err(|_| ErrorKind::ChannelError)?;

        Ok(())
    }

    async fn aps_data_confirm(&mut self) -> Result<()> {
        let response = self.deconz.make_request(Request::ApsDataConfirm).await?;
        let (request_id, aps_data_confirm) = match response {
            Response::ApsDataConfirm {
                request_id,
                aps_data_confirm,
                ..
            } => (request_id, aps_data_confirm),
            resp => return Err(ErrorKind::UnexpectedResponse(resp.command_id()).into()),
        };

        self.route_confirm(request_id, aps_data_confirm).await?;

        Ok(())
    }

    fn request_id(&mut self) -> RequestId {
        let old = self.request_id;
        self.request_id += 1;
        old
    }

    async fn aps_data_request(&mut self, request: ApsDataRequest) -> Result<RequestId> {
        let request_id = self.request_id();
        let request = Request::ApsDataRequest(request_id, request);
        let response = self.deconz.make_request(request).await?;

        // We don't bother checking the request_id in the response, as the
        // sequence_id should be sufficient.
        if !matches!(response, Response::ApsDataRequest { .. }) {
            return Err(ErrorKind::UnexpectedResponse(response.command_id()).into());
        }

        Ok(request_id)
    }

    async fn route_confirm(
        &mut self,
        request_id: RequestId,
        aps_data_confirm: ApsDataConfirm,
    ) -> Result<()> {
        match self.awaiting.remove(&request_id) {
            Some(sender) => sender
                .send(Ok(aps_data_confirm))
                .map_err(|_| ErrorKind::ChannelError)?,
            None => {
                error!("don't know where to route response");
            }
        };
        Ok(())
    }
}

pub struct ApsReader {
    pub(crate) rx: mpsc::Receiver<ApsDataIndication>,
}

impl Stream for ApsReader {
    type Item = ApsDataIndication;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}
