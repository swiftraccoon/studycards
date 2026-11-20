//! The web application: its routes and the actions behind them.
//!
//! [`App`] answers requests from the files of a
//! [`Store`](crate::store::Store). Every page is rendered on the server. A
//! choice on a study page or an edit arrives as a form post, changes the
//! files, and redirects to the page that shows the result, so reloading a page
//! never repeats an action.
//!
//! | Path | Purpose |
//! | --- | --- |
//! | `/` | The list of sets; posting to `/sets` creates one. |
//! | `/sets/{set}` | Overview of a set. |
//! | `/sets/{set}/study` | Study all cards. |
//! | `/sets/{set}/study/{topic}` | Study the cards of one topic. |
//! | `/sets/{set}/edit` | Edit the set, its topics, and its cards. |
//! | `/assets/…` | The style sheet, the icon, and the keyboard script. |
//!
//! # Examples
//!
//! Serving the sets of the directory `data` on port 8717. The example is not
//! run as a test, because the server runs until the process ends.
//!
//! ```no_run
//! use std::path::Path;
//!
//! use studycards::http::Server;
//! use studycards::store::Store;
//! use studycards::web::App;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = Store::open(Path::new("data"))?;
//! let server = Server::bind(8717)?;
//! println!("Listening at http://127.0.0.1:{}/", server.port());
//! server.run(App::new(store))?;
//! # Ok(())
//! # }
//! ```

pub use self::app::App;

mod app;
mod element;
mod pages;
