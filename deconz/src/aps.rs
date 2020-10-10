use tokio::stream::Stream;
use tokio::sync::{mpsc, oneshot, watch};
use tophamm_helpers::awaiting;

use crate::protocol::RequestId;
use crate::{
    ApsDataConfirm, ApsDataIndication, ApsDataRequest, Deconz, DeviceState, Error, ErrorKind,
    Request, Response, Result,
};

pub type Awaiting = awaiting::Awaiting<RequestId, ApsDataConfirm, Error>;

/// A command from Deconz to the Aps task, representing an ApsDataRequest.
pub type ApsRequest = (
    RequestId,
    ApsDataRequest,
    oneshot::Sender<Result<ApsDataConfirm>>,
);

/// Task responsible for forwarding ApsDataRequests to the adapter.
pub struct ApsRequests {
    pub deconz: Deconz,
    pub device_state: watch::Receiver<DeviceState>,
    pub awaiting: Awaiting,
    pub requests: mpsc::Receiver<ApsRequest>,
}

impl ApsRequests {
    pub async fn task(mut self) -> Result<()> {
        // Wait until the device tells us that it's ready to receive requests.
        let mut request_free_slots = false;

        loop {
            tokio::select! {
                Some(device_state) = self.device_state.recv() => {
                    request_free_slots = device_state.data_request_free_slots;
                }
                Some((id, request, sender)) = self.requests.recv(),
                    if request_free_slots =>
                {
                    // Assume we can only send one message at a time. We'll get a DeviceState in
                    // the response which will tell us if we can send more.
                    request_free_slots = false;

                    let awaiting = self.awaiting.clone();
                    let future = self.forward_request(id, request);
                    awaiting.register_while(id, sender, future).await;
                }
                else => break,
            }
        }

        Ok(())
    }

    async fn forward_request(
        &mut self,
        request_id: RequestId,
        request: ApsDataRequest,
    ) -> Result<()> {
        let request = Request::ApsDataRequest(request_id, request);
        let response = self.deconz.make_request(request).await?;

        // We don't bother checking the request_id in the response, as the
        // sequence_id should be sufficient.
        if !matches!(response, Response::ApsDataRequest { .. }) {
            return Err(ErrorKind::UnexpectedResponse(response.command_id()).into());
        }

        Ok(())
    }
}

/// Task responsible for querying the adapter for ApsDataConfirms and forwarding them to whomever
/// is awaiting confirmation.
pub struct ApsConfirms {
    pub deconz: Deconz,
    pub device_state: watch::Receiver<DeviceState>,
    pub awaiting: Awaiting,
}

impl ApsConfirms {
    pub async fn task(mut self) -> Result<()> {
        while let Some(device_state) = self.device_state.recv().await {
            if device_state.data_confirm {
                if let Err(error) = self.aps_data_confirm().await {
                    error!("aps_data_confirm: {}", error);
                }
            }
        }
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

        if let Some(_) = self.awaiting.send(&request_id, Ok(aps_data_confirm)) {
            return Err(ErrorKind::UnsolicitedConfirm(request_id).into());
        }

        Ok(())
    }
}

/// Task responsible for querying the adapter for ApsDataIndications and forwarding to the
/// application.
pub struct ApsIndications {
    pub deconz: Deconz,
    pub device_state: watch::Receiver<DeviceState>,
    pub aps_data_indications: mpsc::Sender<ApsDataIndication>,
}

impl ApsIndications {
    pub async fn task(mut self) -> Result<()> {
        while let Some(device_state) = self.device_state.recv().await {
            if device_state.data_indication {
                let aps_data_indication = match self.aps_data_indication().await {
                    Ok(aps_data_indication) => aps_data_indication,
                    Err(error) => {
                        error!("aps_data_indication: {}", error);
                        continue;
                    }
                };

                if let Err(_) = self.aps_data_indications.send(aps_data_indication).await {
                    // The receiver has been dropped - no point continuing.
                    break;
                }
            }
        }
        Ok(())
    }

    async fn aps_data_indication(&mut self) -> Result<ApsDataIndication> {
        let response = self.deconz.make_request(Request::ApsDataIndication).await?;
        let aps_data_indication = match response {
            Response::ApsDataIndication {
                aps_data_indication,
                ..
            } => aps_data_indication,
            resp => return Err(ErrorKind::UnexpectedResponse(resp.command_id()).into()),
        };
        Ok(aps_data_indication)
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
