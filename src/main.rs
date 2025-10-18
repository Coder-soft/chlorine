use eframe::egui;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use poll_promise::Promise;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AppSettings {
    dark_mode: bool,
    hotkey: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            hotkey: "Ctrl+Shift+C".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageInfo {
    filename: String,
    relative_path: String,
    full_path: String,
    extension: String,
    size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Category {
    directory: String,
    images: Vec<ImageInfo>,
    count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageData {
    categories: HashMap<String, Category>,
}

struct ImageSearchApp {
    image_data: Option<ImageData>,
    search_query: String,
    selected_category: String,
    filtered_images: Vec<(String, ImageInfo)>,
    selected_image: Option<(String, ImageInfo)>,
    show_all_categories: bool,
    loaded_textures: HashMap<String, egui::TextureHandle>,
    loading_promises: HashMap<String, Promise<Option<egui::ColorImage>>>,
    failed_images: std::collections::HashSet<String>,
    status_message: String,
    settings: AppSettings,
    show_settings: bool,
}

impl Default for ImageSearchApp {
    fn default() -> Self {
        let mut app = Self {
            image_data: None,
            search_query: String::new(),
            selected_category: "All Categories".to_string(),
            filtered_images: Vec::new(),
            selected_image: None,
            show_all_categories: true,
            loaded_textures: HashMap::new(),
            loading_promises: HashMap::new(),
            failed_images: std::collections::HashSet::new(),
            status_message: "Loading image list...".to_string(),
            settings: AppSettings::default(),
            show_settings: false,
        };
        app.load_image_data();
        app
    }
}

impl ImageSearchApp {
    fn load_image_data(&mut self) {
        if let Ok(content) = std::fs::read_to_string("image_list.json") {
            match serde_json::from_str::<ImageData>(&content) {
                Ok(data) => {
                    self.image_data = Some(data);
                    self.update_filtered_images();
                    self.status_message = format!("Loaded {} categories", 
                        self.image_data.as_ref().unwrap().categories.len());
                }
                Err(e) => {
                    self.status_message = format!("Error parsing JSON: {}", e);
                }
            }
        } else {
            let cwd = std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            self.status_message = format!("Error: Could not read image_list.json from: {}", cwd);
        }
    }

    fn update_filtered_images(&mut self) {
        if let Some(data) = &self.image_data {
            self.filtered_images.clear();
            
            for (category_name, category) in &data.categories {
                if self.show_all_categories || self.selected_category == *category_name {
                    for image in &category.images {
                        let search_lower = self.search_query.to_lowercase();
                        let filename_lower = image.filename.to_lowercase();
                        let category_lower = category_name.to_lowercase();
                        
                        let matches_search = self.search_query.is_empty() ||
                            filename_lower.starts_with(&search_lower) ||  // First letter match
                            filename_lower.contains(&search_lower) ||     // Contains match
                            category_lower.contains(&search_lower);       // Category match
                        
                        if matches_search {
                            self.filtered_images.push((category_name.clone(), image.clone()));
                        }
                    }
                }
            }
            
            // Sort once after filtering
            self.filtered_images.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.filename.cmp(&b.1.filename)));
        }
    }

    fn load_image_texture(&mut self, ctx: &egui::Context, image_info: &ImageInfo) -> Option<egui::TextureHandle> {
        let path = image_info.full_path.clone();
        
        // Check if already loaded
        if let Some(texture) = self.loaded_textures.get(&path) {
            return Some(texture.clone());
        }

        // Check if failed before
        if self.failed_images.contains(&path) {
            return None;
        }

        // Check if currently loading
        if let Some(promise) = self.loading_promises.get(&path) {
            if let Some(result) = promise.ready() {
                // Loading complete, create texture
                if let Some(color_image) = result {
                    let texture = ctx.load_texture(
                        &path,
                        color_image.clone(),
                        egui::TextureOptions::default(),
                    );
                    self.loaded_textures.insert(path.clone(), texture.clone());
                    self.loading_promises.remove(&path);
                    return Some(texture);
                } else {
                    // Loading failed
                    self.loading_promises.remove(&path);
                    self.failed_images.insert(path);
                    return None;
                }
            } else {
                // Still loading, request repaint
                ctx.request_repaint();
                return None;
            }
        }

        // Limit concurrent loads to prevent thread explosion
        const MAX_CONCURRENT_LOADS: usize = 10;
        if self.loading_promises.len() >= MAX_CONCURRENT_LOADS {
            return None;
        }

        // Start loading in background thread
        let path_clone = path.clone();
        let promise = Promise::spawn_thread("load_image", move || {
            if !Path::new(&path_clone).exists() {
                return None;
            }
            
            let image_data = std::fs::read(&path_clone).ok()?;
            let img = image::load_from_memory(&image_data).ok()?;
            
            // Resize to thumbnail (max 128x128) for better performance
            let thumbnail = img.thumbnail(128, 128);
            let rgba = thumbnail.to_rgba8();
            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = rgba.into_raw();
            
            Some(egui::ColorImage::from_rgba_unmultiplied(
                size,
                &pixels,
            ))
        });
        
        self.loading_promises.insert(path, promise);
        ctx.request_repaint();
        None
    }

    fn copy_image_to_clipboard(&mut self, image_info: &ImageInfo) {
        if Path::new(&image_info.full_path).exists() {
            if let Ok(image_data) = std::fs::read(&image_info.full_path) {
                if let Ok(img) = image::load_from_memory(&image_data) {
                    if let Some(rgba) = img.as_rgba8() {
                        match arboard::Clipboard::new() {
                            Ok(mut clipboard) => {
                                match clipboard.set_image(arboard::ImageData {
                                    width: rgba.width() as usize,
                                    height: rgba.height() as usize,
                                    bytes: std::borrow::Cow::Borrowed(rgba.as_raw()),
                                }) {
                                    Ok(_) => {
                                        self.status_message = format!("Copied {} to clipboard", image_info.filename);
                                    }
                                    Err(e) => {
                                        self.status_message = format!("Failed to copy to clipboard: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                self.status_message = format!("Failed to access clipboard: {}", e);
                            }
                        }
                    }
                }
            } else {
                self.status_message = format!("Image file not found: {}", image_info.full_path);
            }
        }
    }
}

impl eframe::App for ImageSearchApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply theme
        if self.settings.dark_mode {
            ctx.set_visuals(egui::Visuals::dark());
        } else {
            ctx.set_visuals(egui::Visuals::light());
        }
        
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.add_space(10.0);
            
            ui.horizontal(|ui| {
                ui.heading("Chlorine");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙️ Settings").clicked() {
                        self.show_settings = !self.show_settings;
                    }
                    ui.add_space(10.0);
                    ui.label(&self.status_message);
                });
            });
            
            ui.add_space(5.0);
            
            ui.horizontal(|ui| {
                ui.label("Search:");
                let response = ui.add_sized(
                    [300.0, 24.0],
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Search by filename or category..."),
                );
                
                if response.changed() {
                    self.update_filtered_images();
                }
                
                if let Some(data) = &self.image_data {
                    let mut categories: Vec<String> = data.categories.keys().cloned().collect();
                    categories.sort();
                    categories.insert(0, "All Categories".to_string());
                    
                    ui.label("Category:");
                    let prev_category = self.selected_category.clone();
                    egui::ComboBox::from_label("")
                        .selected_text(&self.selected_category)
                        .show_ui(ui, |ui| {
                            for category in &categories {
                                ui.selectable_value(&mut self.selected_category, category.clone(), category);
                            }
                        });
                    
                    // Update filter when category changes
                    if prev_category != self.selected_category {
                        self.show_all_categories = self.selected_category == "All Categories";
                        self.update_filtered_images();
                    }
                    
                    if ui.button("🔄 Refresh").clicked() {
                        self.load_image_data();
                    }
                }
            });
            
            ui.add_space(10.0);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(format!("Found {} images", self.filtered_images.len()));
            
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show_rows(ui, 80.0, self.filtered_images.len(), |ui, row_range| {
                    for i in row_range {
                        if let Some((category, image_info)) = self.filtered_images.get(i) {
                            let category = category.clone();
                            let image_info = image_info.clone();
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                if let Some(texture) = self.load_image_texture(ctx, &image_info) {
                                    ui.image((texture.id(), egui::Vec2::new(64.0, 64.0)));
                                } else {
                                    // Show spinner while loading
                                    ui.allocate_ui(egui::Vec2::new(64.0, 64.0), |ui| {
                                        ui.centered_and_justified(|ui| {
                                            ui.spinner();
                                        });
                                    });
                                }
                                
                                ui.vertical(|ui| {
                                    ui.strong(&image_info.filename);
                                    ui.label(format!("📁 {}", category));
                                    ui.label(format!("📊 {} KB", image_info.size / 1024));
                                    ui.label(format!("📍 {}", image_info.relative_path));
                                });
                                
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button("📋 Copy Image").clicked() {
                                        self.copy_image_to_clipboard(&image_info);
                                    }
                                    
                                    if ui.button("👁️ View Details").clicked() {
                                        self.selected_image = Some((category.clone(), image_info.clone()));
                                    }
                                });
                            });
                        });
                        
                        ui.add_space(5.0);
                    }
                }
                });
        });

        if let Some((category, image_info)) = &self.selected_image {
            let category = category.clone();
            let image_info = image_info.clone();
            
            egui::Window::new(&image_info.filename)
                .collapsible(false)
                .resizable(true)
                .default_size([500.0, 500.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        // Display image in a square area
                        if let Some(texture) = self.load_image_texture(ctx, &image_info) {
                            let available_width = ui.available_width();
                            let max_size = available_width.min(450.0);
                            
                            // Make it square by using the same dimension for both width and height
                            let scale = (max_size / texture.size_vec2().x).min(max_size / texture.size_vec2().y).min(1.0);
                            let display_size = texture.size_vec2() * scale;
                            
                            ui.add_space(10.0);
                            ui.image((texture.id(), display_size));
                            ui.add_space(10.0);
                        } else {
                            // Show spinner while loading
                            ui.add_space(200.0);
                            ui.spinner();
                            ui.add_space(200.0);
                        }
                        
                        // Show filename and category
                        ui.separator();
                        ui.add_space(5.0);
                        ui.label(egui::RichText::new(&image_info.filename).strong().size(14.0));
                        ui.label(format!("📁 {}", category));
                        ui.add_space(10.0);
                        
                        // Buttons in a horizontal layout
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            
                            if ui.button(egui::RichText::new("📋 Copy").size(16.0)).clicked() {
                                self.copy_image_to_clipboard(&image_info);
                            }
                            
                            ui.add_space(10.0);
                            
                            if ui.button(egui::RichText::new("❌ Close").size(16.0)).clicked() {
                                self.selected_image = None;
                            }
                        });
                        
                        ui.add_space(10.0);
                    });
                });
        }

        // Settings window
        if self.show_settings {
            egui::Window::new("⚙️ Settings")
                .collapsible(false)
                .resizable(false)
                .default_size([400.0, 300.0])
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(10.0);
                    
                    ui.heading("Appearance");
                    ui.add_space(5.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Theme:");
                        if ui.selectable_label(self.settings.dark_mode, "🌙 Dark").clicked() {
                            self.settings.dark_mode = true;
                        }
                        if ui.selectable_label(!self.settings.dark_mode, "☀️ Light").clicked() {
                            self.settings.dark_mode = false;
                        }
                    });
                    
                    ui.add_space(15.0);
                    ui.separator();
                    ui.add_space(15.0);
                    
                    ui.heading("Hotkey");
                    ui.add_space(5.0);
                    
                    ui.horizontal(|ui| {
                        ui.label("Show/Hide Window:");
                        ui.text_edit_singleline(&mut self.settings.hotkey);
                    });
                    
                    ui.label(egui::RichText::new("Note: Hotkey requires app restart").small().weak());
                    
                    ui.add_space(20.0);
                    ui.separator();
                    ui.add_space(10.0);
                    
                    ui.horizontal(|ui| {
                        ui.add_space(80.0);
                        if ui.button(egui::RichText::new("✓ Close").size(16.0)).clicked() {
                            self.show_settings = false;
                        }
                    });
                    
                    ui.add_space(10.0);
                });
        }
    }
}

fn main() -> Result<(), eframe::Error> {
    // Load icon
    let icon_data = load_icon();
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0])
            .with_title("Chlorine")
            .with_icon(icon_data),
        ..Default::default()
    };

    eframe::run_native(
        "Chlorine",
        options,
        Box::new(|_cc| {
            Box::new(ImageSearchApp::default())
        }),
    )
}

fn load_icon() -> egui::IconData {
    let icon_path = "src/clown_logo.png";
    
    // Load and decode the icon
    if let Ok(icon_bytes) = std::fs::read(icon_path) {
        if let Ok(img) = image::load_from_memory(&icon_bytes) {
            let rgba = img.to_rgba8();
            let (width, height) = (rgba.width(), rgba.height());
            
            return egui::IconData {
                rgba: rgba.into_raw(),
                width,
                height,
            };
        }
    }
    
    // Fallback: return a minimal 1x1 transparent icon if loading fails
    egui::IconData {
        rgba: vec![0, 0, 0, 0],
        width: 1,
        height: 1,
    }
}
