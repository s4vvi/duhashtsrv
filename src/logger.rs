//
// Create things for logger
//
pub struct Logger;

impl log::Log for Logger {
   fn enabled(&self, _metadata: &log::Metadata) -> bool {
       true
   }

   fn log(&self, record: &log::Record) {
       if !self.enabled(record.metadata()) {
           return;
       }

       println!("[{}]: {}", record.level(), record.args());
   }
   fn flush(&self) {}
}
