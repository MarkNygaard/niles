//! Stopping `niles serve` in order, for whatever reason: a restart asked
//! for in the app, a SIGTERM from Kubernetes or Docker, or Ctrl-C. A
//! restart is this stop followed by an exit with code 0, which whatever
//! runs Niles answers by starting it again.

use niles_automations::AutomationEngine;
use niles_mqtt::MqttPublisher;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

/// Why Niles is stopping. Kept for the log line that says a stop was
/// seen and answered, whatever brought it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    /// Ctrl-C at the terminal.
    Interrupted,
    /// SIGTERM: Kubernetes or Docker stopping the container.
    Terminated,
    /// Somebody pressed Restart now in the app.
    Restart,
}

impl std::fmt::Display for Stop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Interrupted => write!(f, "Ctrl-C"),
            Self::Terminated => write!(f, "SIGTERM"),
            Self::Restart => write!(f, "a restart asked for in the app"),
        }
    }
}

/// The first reason to stop that comes, or none ever. A failed Ctrl-C
/// registration disables that branch instead of stopping Niles at once.
pub(crate) async fn stopped(mut restart: tokio::sync::watch::Receiver<bool>) -> Stop {
    let restart = async move {
        // A sender gone without asking means nobody can ask any more,
        // not that somebody did.
        if restart.wait_for(|asked| *asked).await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        Ok(()) = tokio::signal::ctrl_c() => Stop::Interrupted,
        () = terminated() => Stop::Terminated,
        () = restart => Stop::Restart,
    }
}

#[cfg(unix)]
async fn terminated() {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut signal) => {
            signal.recv().await;
        }
        Err(e) => {
            tracing::warn!("[shutdown] cannot listen for SIGTERM: {e}");
            std::future::pending::<()>().await;
        }
    }
}

#[cfg(not(unix))]
async fn terminated() {
    // The dev machine is Windows; SIGTERM does not exist there.
    std::future::pending::<()>().await;
}

/// The order of a stop. Each variant's place is the reason it sits
/// where it does; see the ARCHITECTURE.md section "Restarting Niles".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Stage {
    /// Nothing new is taken: the API stops accepting, the Wyoming
    /// server stops hearing satellites.
    StopIntake,
    /// What a request in flight is doing finishes: a save lands before
    /// its Niles goes away.
    FinishRequests,
    /// No run starts, and a run waiting out a delay keeps its wait.
    /// Before the broker closes, so a delay that ends now cannot run
    /// into a closed broker and then forget its wait.
    StopAutomations,
    /// Add-ons stop before the speech they may be speaking does.
    StopAddOns,
    /// Speaking stops before the satellites it plays through close.
    StopSpeaking,
    /// Satellites' connections end, each socket closed rather than
    /// reset when the process exits.
    CloseSatellites,
    /// Last: everything a run or a page may have been about to say
    /// has been said.
    CloseMqtt,
}

type Step = Pin<Box<dyn Future<Output = ()> + Send>>;

/// An ordered stop: steps registered against stages, run in `Stage`
/// order whatever order they were added, each within its budget.
#[derive(Default)]
pub(crate) struct Shutdown {
    steps: Vec<(Stage, Duration, Step)>,
}

impl Shutdown {
    pub(crate) fn at(
        &mut self,
        stage: Stage,
        budget: Duration,
        step: impl Future<Output = ()> + Send + 'static,
    ) {
        self.steps.push((stage, budget, Box::pin(step)));
    }

    /// Run the steps in `Stage` order, leaving a step that runs over
    /// its budget behind. Returns the stages that ran, for the log and
    /// the tests.
    pub(crate) async fn run(mut self) -> Vec<Stage> {
        // Stable, so steps within a stage keep the order they were
        // added.
        self.steps.sort_by_key(|(stage, _, _)| *stage);
        let mut ran = Vec::new();
        for (stage, budget, step) in self.steps {
            if tokio::time::timeout(budget, step).await.is_err() {
                tracing::warn!("[shutdown] {stage:?} ran over {budget:?}; going on without it");
            }
            ran.push(stage);
        }
        ran
    }
}

/// What `serve` was running and must come down in order.
pub(crate) struct Parts<Prepared: Send + 'static> {
    pub(crate) api: JoinHandle<()>,
    pub(crate) api_stop: tokio::sync::oneshot::Sender<()>,
    pub(crate) wyoming_server: JoinHandle<()>,
    pub(crate) satellites: niles_wyoming::WyomingSender,
    pub(crate) automations: Arc<AutomationEngine>,
    pub(crate) plugins: Option<niles_plugins::Runtime>,
    pub(crate) speech: Option<Arc<niles_tts::speech_session::SpeechSessionManager<Prepared>>>,
    pub(crate) mqtt: MqttPublisher,
    pub(crate) wled: Option<(JoinHandle<()>, MqttPublisher)>,
    /// Everything else still running, stopped last.
    pub(crate) rest: Vec<JoinHandle<()>>,
}

/// Stop what `serve` was running, in `Stage` order. A step that runs
/// over its budget is left behind; the whole stop takes at most about
/// ten seconds, within Docker's default and Kubernetes' 30 s.
pub(crate) async fn stop<Prepared: Send + 'static>(parts: Parts<Prepared>) {
    const BRIEF: Duration = Duration::from_secs(1);
    let Parts {
        api,
        api_stop,
        wyoming_server,
        satellites,
        automations,
        plugins,
        speech,
        mqtt,
        wled,
        mut rest,
    } = parts;
    let api_abort = api.abort_handle();
    let wled_mqtt = wled.map(|(handle, publisher)| {
        rest.push(handle);
        publisher
    });

    let mut shutdown = Shutdown::default();
    shutdown.at(Stage::StopIntake, BRIEF, async move {
        let _ = api_stop.send(());
        wyoming_server.abort();
    });
    shutdown.at(Stage::FinishRequests, Duration::from_secs(4), async move {
        let _ = api.await;
    });
    shutdown.at(Stage::StopAutomations, BRIEF, async move {
        automations.stop();
    });
    if let Some(runtime) = plugins {
        shutdown.at(Stage::StopAddOns, Duration::from_secs(3), async move {
            runtime.shutdown().await;
        });
    }
    if let Some(manager) = speech {
        shutdown.at(Stage::StopSpeaking, BRIEF, async move {
            manager.shutdown().await;
        });
    }
    shutdown.at(Stage::CloseSatellites, BRIEF, async move {
        satellites.close_all();
    });
    shutdown.at(Stage::CloseMqtt, BRIEF, async move {
        let _ = mqtt.disconnect().await;
        if let Some(wled_mqtt) = wled_mqtt {
            let _ = wled_mqtt.disconnect().await;
        }
        // Queued like a publish: a beat for the eventloops to send it
        // before they are stopped below.
        tokio::time::sleep(Duration::from_millis(250)).await;
    });

    let ran = shutdown.run().await;
    tracing::info!("[shutdown] stopped in order: {ran:?}");
    api_abort.abort();
    for handle in rest {
        handle.abort();
    }
}

/// What a restart would cut short right now, answered from the running
/// parts for `GET /system/restart`'s confirmation.
pub(crate) struct UnderWayNow {
    pub(crate) approvals: Arc<crate::approvals::Approvals>,
    pub(crate) wake_ups: Arc<crate::wake_up::WakeUps>,
    pub(crate) artifacts: Option<Arc<crate::artifacts::Artifacts>>,
}

impl niles_api::UnderWay for UnderWayNow {
    fn under_way(&self) -> niles_api::CutShort {
        niles_api::CutShort {
            jobs: self.artifacts.as_ref().map_or(0, |a| a.jobs_going()),
            wake_ups: self.wake_ups.ramping(),
            approvals: self.approvals.waiting_count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test(start_paused = true)]
    async fn the_stages_run_in_their_order_whatever_order_they_were_added() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut shutdown = Shutdown::default();
        for stage in [
            Stage::CloseMqtt,
            Stage::CloseSatellites,
            Stage::StopSpeaking,
            Stage::StopAddOns,
            Stage::StopAutomations,
            Stage::FinishRequests,
            Stage::StopIntake,
        ] {
            let order = order.clone();
            shutdown.at(stage, Duration::from_secs(1), async move {
                order.lock().unwrap_or_else(|e| e.into_inner()).push(stage);
            });
        }
        let ran = shutdown.run().await;
        assert_eq!(
            ran,
            [
                Stage::StopIntake,
                Stage::FinishRequests,
                Stage::StopAutomations,
                Stage::StopAddOns,
                Stage::StopSpeaking,
                Stage::CloseSatellites,
                Stage::CloseMqtt,
            ]
        );
        let order = order.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(order.len(), 7);
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "added backwards, ran forwards: {order:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_step_that_runs_over_is_left_behind() {
        let ran_later = Arc::new(Mutex::new(false));
        let mut shutdown = Shutdown::default();
        shutdown.at(
            Stage::StopAddOns,
            Duration::from_secs(3),
            std::future::pending(),
        );
        let later = ran_later.clone();
        shutdown.at(Stage::CloseMqtt, Duration::from_secs(1), async move {
            *later.lock().unwrap_or_else(|e| e.into_inner()) = true;
        });
        let ran = shutdown.run().await;
        assert_eq!(ran, [Stage::StopAddOns, Stage::CloseMqtt]);
        assert!(
            *ran_later.lock().unwrap_or_else(|e| e.into_inner()),
            "a later stage still runs after an over-budget one"
        );
    }

    #[test]
    fn automations_stop_before_the_broker_closes() {
        assert!(
            Stage::StopAutomations < Stage::CloseMqtt,
            "a delay ending mid-shutdown must not run into a closed broker and forget its wait"
        );
        assert!(
            Stage::StopIntake < Stage::FinishRequests,
            "requests in flight are waited for, so nothing new may start first"
        );
        assert!(
            Stage::FinishRequests < Stage::StopAutomations,
            "a save's delayed side must land before its engine is told to stop"
        );
    }

    #[tokio::test]
    async fn a_restart_request_stops_the_loop() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        tx.send_replace(true);
        assert_eq!(stopped(rx).await, Stop::Restart);
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_request_sender_is_not_a_restart() {
        let (tx, rx) = tokio::sync::watch::channel(false);
        drop(tx);
        let outcome = tokio::time::timeout(Duration::from_millis(100), stopped(rx)).await;
        assert!(
            outcome.is_err(),
            "nobody left to ask is not an ask: the loop goes on"
        );
    }
}
