#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;

use esp_hal::gpio::Input;
use hakkaa::board::Board;
use hakkaa::led::Storeys;
use hakkaa::switch::LowActiveSwitch;

extern crate alloc;

type ButtonSignal = Signal<CriticalSectionRawMutex, ()>;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

/// A convenience wrapper providing a simple delay.
async fn delay(duration: Duration) {
    Timer::after(duration).await;
}

/// Waits for a single press of `button` with input debouncing.
async fn wait_for_button<'a>(button: &mut Input<'a>) {
    let debounce_delay = Duration::from_millis(100);
    log::debug!("waiting for switch");

    log::debug!("waiting for high");
    button.wait_for_high().await;
    delay(debounce_delay).await;
    log::debug!("waiting for low");
    button.wait_for_low().await;
    delay(debounce_delay).await;
    log::debug!("waiting for high again");
    button.wait_for_high().await;
}

/// Waits for `n` presses of `button`.
async fn wait_for_button_n_times<'a>(button: &mut Input<'a>, n: usize) {
    for _ in 0..n {
        wait_for_button(button).await;
    }
}

/// Task waiting for three times an input on `button` and signalling this event through `signal`.
#[embassy_executor::task(pool_size = 2)]
async fn button_task(mut button: Input<'static>, signal: &'static ButtonSignal) {
    loop {
        wait_for_button(&mut button).await;
        signal.signal(());
    }
}

/// Task performing the board by orchestrating LED patterns and checking button inputs.
#[embassy_executor::task]
async fn pomodoro_task(
    mut storeys: Storeys<'static>,
    first_button: &'static ButtonSignal,
    mut finished_led: LowActiveSwitch<'static>,
) {
    let step = Duration::from_secs(1);
    let pomodoro_timer = Duration::from_secs(60 * 25);
    let break_timer = Duration::from_secs(60 * 5);

    // TODO: currently timer starts immediately.
    log::info!("Pomodoro Timer: Press the button three times to start a 25 minute timer.");
    first_button.reset();
    //    first_button.wait().await;

    match select(storeys.cycle(step), timer(pomodoro_timer)).await {
        Either::First(_) => log::debug!("cycle done"),
        Either::Second(_) => log::debug!("timer done"),
    }

    log::info!("Pomodoro timer finished! Taking a short 5 minute break.",);
    match select(storeys.blink(step), timer(break_timer)).await {
        Either::First(_) => log::debug!("blink done"),
        Either::Second(_) => log::debug!("break timer done"),
    };

    finished_led.switch_off();

    log::info!("Press Ctrl + C to exit.");
}

async fn timer(minutes: Duration) {
    Timer::after(minutes).await
}

static SW1_SIGNAL: ButtonSignal = ButtonSignal::new();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let board = Board::init();

    let storeys = Storeys::new(board.storey_leds);

    log::info!("Starting Pomodoro Timer.");

    // 1. Wait for user to press SW1 button.
    // 2. Start blinking cycle on LEDs for 25 minutes. (done in pomodoro_task)
    // 3. Blink all LEDs rapidly for 5 minutes. (done in pomodoro_task)
    // 4. Repeat from 1.

    // Press SW1 two times to restart the pomodoro timer.

    // Spawn a debouncing and counting task for each "button". Each triplet of "presses" will
    // generate as signal which is later checked by the EOL task.
    spawner.spawn(button_task(board.sw1, &SW1_SIGNAL)).unwrap();

    // Finally spawn the EOL task showing different storey LED patterns for user inspection of LEDs
    // and as a prompt for pressing SW1 or shaking the board for checking the shake sensor U2.
    spawner
        .spawn(pomodoro_task(storeys, &SW1_SIGNAL, board.esp_led))
        .unwrap();

    // Keep the main task running forever.
    loop {
        delay(Duration::from_secs(3)).await;
    }
}
