use rfd::FileDialog;
use std::{fs, sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc, Mutex }, time::Duration};
use slint::{ComponentHandle, Model, ModelRc, SharedString};
use std::rc::Rc;
use slint::VecModel;
use std::io;
use rodio::{Decoder, OutputStream, Sink, Source};

slint::include_modules!();

#[derive(Debug)]
enum State {
    Playing,
    Stop,
    Pause,
}

enum Event {
    TotalDuration,
}

fn round(num: f32) -> f32 {
    let rounded_num = (num * 10.0).round() / 10.0;

    return rounded_num;
}

fn format_duration(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    
    return format!("{:02}:{:02}", minutes, seconds);
}

fn main() -> Result<(), slint::PlatformError> {
    let const_init_volume = round(0.1);
    let max_list_count = Arc::new(Mutex::new(0));

    let (tx_total_duration, rx_total_duration) = mpsc::channel::<u64>();

    let ui = AppWindow::new()?;
    let ui_weak = ui.as_weak();
    let running = Arc::new(AtomicBool::new(true));

    let running_clone = Arc::clone(&running);
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    
    let sink = Sink::try_new(&stream_handle).expect("Failed to create sink");

    sink.set_volume(const_init_volume);
    ui.set_volume((const_init_volume * 100.0) as i32);
    ui.set_expanded(false);

    let arc_sink = Arc::new(sink);
    let arc_ui = Arc::new(ui_weak);

    let sink_clone = Arc::clone(&arc_sink);
    let mut total_duration_secs = 0;

    let ui_weak = ui.as_weak();
    let ui_weak_clone = ui_weak.clone();

    std::thread::spawn(move || {
        while running_clone.load(Ordering::SeqCst) {
            if !sink_clone.empty() {
                if let Ok(actual_total_duration_secs) = rx_total_duration.try_recv() {
                    total_duration_secs = actual_total_duration_secs;
                }

                let pos = sink_clone.get_pos();
                let formatted_time = format_duration(pos);
                let pos_secs = sink_clone.get_pos().as_secs();

                let progress = (100 * pos_secs) / total_duration_secs;

                let _ = ui_weak_clone.upgrade_in_event_loop(move |window| {
                    window.set_current_duration(SharedString::from(formatted_time.to_string()));
                    window.set_width_percentage(progress as i32);
                });
            }
            
            if sink_clone.empty() {
                 let _ = ui_weak_clone.upgrade_in_event_loop(move |window| {
                    window.set_current_duration(SharedString::from("00:00".to_string()));
                    window.set_width_percentage(0);
                });

                sink_clone.stop();

                total_duration_secs = 0;
            }

            std::thread::sleep(Duration::from_millis(100));

        }
    });

    // std::thread::spawn(move || {
    //     while running_clone.load(Ordering::SeqCst) {
    //         let pos = sink_clone.get_pos();

    //         let formatted_time = format_duration(pos);
            
    //         if let Ok(temp) = rx.recv() {
    //             match temp {
    //                 State::Playing => {
    //                     let _ = ui_weak_clone.upgrade_in_event_loop(move |window| {
    //                         window.set_current_duration(SharedString::from(formatted_time.to_string()));

    //                         // sink_clone.get_pos()
    //                     });
    //                 }
    //                 State::Stop => {
    //                     let _ = ui_weak_clone.upgrade_in_event_loop(move |window| {
    //                         window.set_current_duration(SharedString::from("00:00".to_string()));
    //                     });
    //                 }
    //                 State::Pause => {
    //                     println!("Pause");
    //                 }
    //             }
    //         }
    //         println!("main thread");
    //         std::thread::sleep(Duration::from_millis(100));
    //     }
       
    // });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let max_list_count_clone = Arc::clone(&max_list_count);

    ui.on_selectPath(move || {
        if let Some(path) = FileDialog::new().pick_folder() {
            let path = path.to_string_lossy().to_string();
            
            ui_weak_clone.unwrap().set_path(SharedString::from(path.clone()));
            
            let paths = fs::read_dir(path).unwrap();
            let mut file_list = vec![];
            
            let mut counter = 0;

            for path in paths {
                let path = path.unwrap().path();

                if let Some(extension) = path.extension() {
                    if extension == "mp3" {
                        let file_duration = match mp3_duration::from_path(&path) {
                            Ok(duration) => format_duration(duration),
                            Err(_) => "Unknown".to_string(),
                        };

                        let record: Record = Record {
                            id: counter as i32,
                            filePath: SharedString::from(path.to_string_lossy().to_string()),
                            fileName: SharedString::from(path.file_stem().unwrap().to_string_lossy().to_string()),
                            duration: SharedString::from(file_duration.to_string()),
                        };

                        file_list.push(record);

                        counter = counter + 1;
                    }
                }
            }

            let mut max_list_count = max_list_count_clone.lock().unwrap();

            *max_list_count = counter;

            let file_list_model = Rc::new(VecModel::from(file_list));

            ui_weak_clone.unwrap().set_list(file_list_model.into());
        }
    });   

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);
    let tx_total_duration_clone = tx_total_duration.clone();

    ui.on_selectRecord(move |id, text| {
        let playing_id = ui_weak_clone.unwrap().get_idPlaying();

        let is_playing = ui_weak_clone.unwrap().get_isPlaying();
        let is_pausing = ui_weak_clone.unwrap().get_isPausing();

        if playing_id == id {
            if is_playing == true {
                sink_clone.pause();
    
                ui_weak_clone.unwrap().set_isPausing(true);
                ui_weak_clone.unwrap().set_isPlaying(false);
    
                return;
            }
            
            if is_pausing == true {
                sink_clone.play();

                ui_weak_clone.unwrap().set_isPausing(false);
                ui_weak_clone.unwrap().set_isPlaying(true);

                return;
            }
        }

        sink_clone.stop();

        let file = match fs::File::open(text.to_string()) {
            Ok(file) => file,
            Err(e) => {
                println!("Failed to open file: {}", e);

                return;
            }
        };
        
        let source = match Decoder::new(io::BufReader::new(file)) {
            Ok(source) => source,
            Err(e) => {
                println!("Failed to decode audio: {}", e);
                return;
            }
        };
        

        let total_duration = source.total_duration().unwrap();
        let formatted_time = format_duration(total_duration);

        ui_weak_clone.unwrap().set_total_duration(SharedString::from(formatted_time.to_string()));
        
        sink_clone.append(source);
        sink_clone.play();

        ui_weak_clone.unwrap().set_idPlaying(id as i32);
        ui_weak_clone.unwrap().set_idPausing(id as i32);

        ui_weak_clone.unwrap().set_isPlaying(true);
        ui_weak_clone.unwrap().set_isPausing(false);
        ui_weak_clone.unwrap().set_expanded(true);

        let _ = tx_total_duration_clone.send(total_duration.as_secs());
        
    });
       
    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);

    ui.on_pause(move || {
        let is_playing = ui_weak_clone.unwrap().get_isPlaying();

        if is_playing == true {
            sink_clone.pause();

            ui_weak_clone.unwrap().set_isPausing(true);
            ui_weak_clone.unwrap().set_isPlaying(false);

        }
    });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);
    
    ui.on_play(move || {
        let is_playing = ui_weak_clone.unwrap().get_isPlaying();
        let is_pausing = ui_weak_clone.unwrap().get_isPausing();

        if is_playing == false && is_pausing == true {
            sink_clone.play();

            ui_weak_clone.unwrap().set_isPausing(false);
            ui_weak_clone.unwrap().set_isPlaying(true);
        }
    });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);

    ui.on_stop(move || {
        let is_playing = ui_weak_clone.unwrap().get_isPlaying();

        if is_playing == true {
            sink_clone.stop();
            
            ui_weak_clone.unwrap().set_isPlaying(false);
            ui_weak_clone.unwrap().set_isPausing(false);
            ui_weak_clone.unwrap().set_expanded(false);
            ui_weak_clone.unwrap().set_idPlaying(-1 as i32);
        }
    });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);

    ui.on_volumeUp(move || {
        let mut current_volume = sink_clone.volume();

        current_volume = round(current_volume + 0.1);

        sink_clone.set_volume(current_volume);
        
        if current_volume >= 1.0 {
            sink_clone.set_volume(1.0);

            current_volume = 1.0;
        }

        ui_weak_clone.unwrap().set_volume((current_volume * 100.0) as i32);
    });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);

    ui.on_volumeDown(move || {
        let mut current_volume = sink_clone.volume();

        current_volume = round(current_volume - 0.1);
        
        sink_clone.set_volume(current_volume);

        if current_volume <= 0.0 {       
            sink_clone.set_volume(0.0);

            current_volume = 0.0;
        }

        ui_weak_clone.unwrap().set_volume((current_volume * 100.0) as i32);
    });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);
    let mut prev_volume = 0.0;

    ui.on_volumeMute(move || {
        let current_volume = sink_clone.volume();

        if current_volume != 0.0 {
            prev_volume = current_volume;

        }
        
        let mut actual_volume = 0.0;

        if current_volume == 0.0  {
            actual_volume = prev_volume;
        }

        sink_clone.set_volume(actual_volume);

        let _ = ui_weak_clone.upgrade_in_event_loop(move |window| {
            window.set_volume((actual_volume * 100.0) as i32);
        });
    });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);
    let max_list_count_clone = Arc::clone(&max_list_count);
    let tx_total_duration_clone = tx_total_duration.clone();

    ui.on_prev(move || {
        let max_list_count = max_list_count_clone.lock().unwrap();
        let list: ModelRc<Record> = ui_weak_clone.unwrap().get_list();
        let playing_id = ui_weak_clone.unwrap().get_idPlaying();
        
        let mut prev_id = playing_id - 1;

        if prev_id < 0 {
            prev_id = *max_list_count - 1;
        }

        if let Some(record) = list.row_data(prev_id as usize) {
            let file_path = record.filePath;

            sink_clone.stop();

            let file = match fs::File::open(file_path.to_string()) {
                Ok(file) => file,
                Err(e) => {
                    println!("Failed to open file: {}", e);
    
                    return;
                }
            };
            
            let source = match Decoder::new(io::BufReader::new(file)) {
                Ok(source) => source,
                Err(e) => {
                    println!("Failed to decode audio: {}", e);
                    return;
                }
            };
            
    
            let total_duration = source.total_duration().unwrap();
            let formatted_time = format_duration(total_duration);
    
            ui_weak_clone.unwrap().set_total_duration(SharedString::from(formatted_time.to_string()));
            
            sink_clone.append(source);
            sink_clone.play();
    
            ui_weak_clone.unwrap().set_idPlaying(prev_id as i32);
            ui_weak_clone.unwrap().set_idPausing(prev_id as i32);
        
            ui_weak_clone.unwrap().set_isPlaying(true);
            ui_weak_clone.unwrap().set_isPausing(false);
            ui_weak_clone.unwrap().set_expanded(true);

            let _ = tx_total_duration_clone.send(total_duration.as_secs());
        }
    });

    let ui_weak_clone  = Arc::clone(&arc_ui);
    let sink_clone = Arc::clone(&arc_sink);
    let max_list_count_clone = Arc::clone(&max_list_count);
    let tx_total_duration_clone = tx_total_duration.clone();

    ui.on_next(move || {
        let max_list_count = max_list_count_clone.lock().unwrap();
        let list: ModelRc<Record> = ui_weak_clone.unwrap().get_list();
        let playing_id = ui_weak_clone.unwrap().get_idPlaying();
        
        let mut next_id = playing_id + 1;

        if next_id >= *max_list_count {
            next_id = 0;
        } 

        if let Some(record) = list.row_data(next_id as usize) {
            let file_path = record.filePath;

            sink_clone.stop();

            let file = match fs::File::open(file_path.to_string()) {
                Ok(file) => file,
                Err(e) => {
                    println!("Failed to open file: {}", e);
    
                    return;
                }
            };
            
            let source = match Decoder::new(io::BufReader::new(file)) {
                Ok(source) => source,
                Err(e) => {
                    println!("Failed to decode audio: {}", e);
                    return;
                }
            };
            
    
            let total_duration = source.total_duration().unwrap();
            let formatted_time = format_duration(total_duration);
    
            ui_weak_clone.unwrap().set_total_duration(SharedString::from(formatted_time.to_string()));
            
            sink_clone.append(source);
            sink_clone.play();
    
            ui_weak_clone.unwrap().set_idPlaying(next_id as i32);
            ui_weak_clone.unwrap().set_idPausing(next_id as i32);
        
            ui_weak_clone.unwrap().set_isPlaying(true);
            ui_weak_clone.unwrap().set_isPausing(false);
            ui_weak_clone.unwrap().set_expanded(true);

            let _ = tx_total_duration_clone.send(total_duration.as_secs());
        }
    });

    ui.run()
}
