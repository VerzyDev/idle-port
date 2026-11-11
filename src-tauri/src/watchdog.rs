use crate::{
    alerts,
    health::{Action, Monitor},
    hub::{Hub, RunningSession},
    native,
    stats::Stats,
};
use std::{
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

const TICK: Duration = Duration::from_secs(5);

pub fn spawn(app: AppHandle) {
    thread::spawn(move || {
        let mut stats = Stats::new();
        let mut last = Instant::now();
        let mut carry = Duration::ZERO;
        loop {
            thread::sleep(TICK);
            carry += last.elapsed();
            last = Instant::now();
            let secs = carry.as_secs();
            carry -= Duration::from_secs(secs);
            tick(&app, &mut stats, secs);
        }
    });
}

fn probe(app: &AppHandle, session: &RunningSession) {
    let hub = app.state::<Hub>();
    let monitor = app.state::<Monitor>();
    let words = hub.watch_words();
    let texts: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
    let Ok(result) = native::probe(app, &session.id, &texts) else {
        return;
    };
    for word in monitor.probed(&session.id, result.online, result.found, &words) {
        monitor.log(&session.title, format!("Texto monitorado apareceu: \"{}\"", word.text));
        alerts::send(
            app,
            &format!("word:{}:{}", session.id, word.text),
            &session.title,
            &format!("O texto \"{}\" apareceu na tela.", word.text),
        );
    }
}

fn act(app: &AppHandle, session: &RunningSession, action: Action) {
    let hub = app.state::<Hub>();
    let monitor = app.state::<Monitor>();
    match action {
        Action::Reload { reason, needs_attention } => {
            let result = hub.reload(app, &session.id);
            monitor.log(
                &session.title,
                match result {
                    Ok(()) => format!("Recarregada automaticamente: {reason}"),
                    Err(e) => format!("Falha ao recarregar ({reason}): {e}"),
                },
            );
            if needs_attention {
                alerts::send(
                    app,
                    &format!("attention:{}", session.id),
                    &session.title,
                    &format!("Precisa de atenção: não se recuperou após várias tentativas ({reason})."),
                );
            }
        }
        Action::Respawn { reason } => {
            let result = hub.respawn(app, &session.id);
            monitor.log(
                &session.title,
                match result {
                    Ok(()) => format!("Reaberta automaticamente: {reason}"),
                    Err(e) => format!("Falha ao reabrir ({reason}): {e}"),
                },
            );
        }
        Action::Attention(reason) => {
            alerts::send(
                app,
                &format!("attention:{}", session.id),
                &session.title,
                &format!("Precisa de atenção: {reason}."),
            );
        }
    }
}

fn tick(app: &AppHandle, stats: &mut Stats, elapsed_secs: u64) {
    let hub = app.state::<Hub>();
    let monitor = app.state::<Monitor>();
    let running = hub.running_sessions();
    let ids: Vec<String> = running.iter().map(|s| s.id.clone()).collect();
    let usage = stats.sample(&ids);

    for session in &running {
        if let Some((memory, cpu)) = usage.get(&session.id) {
            monitor.set_usage(&session.id, *memory, *cpu);
        }
        probe(app, session);
        if let Some(action) = monitor.evaluate(&session.id, session.auto_recover) {
            act(app, session, action);
        }
    }

    let playing: Vec<String> = ids.iter().filter(|id| monitor.is_ok(id)).cloned().collect();
    hub.add_playtime(&playing, elapsed_secs);

    let mut overview = monitor.overview(&ids);
    overview.playtime = hub.playtimes();
    let _ = app.emit("overview", overview);
}
