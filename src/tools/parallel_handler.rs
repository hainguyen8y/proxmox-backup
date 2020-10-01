use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{bail, format_err, Error};
use crossbeam_channel::{bounded, Sender};

/// A handle to send data to the worker thread (implements clone)
pub struct SendHandle<I> {
    input: Sender<I>,
    abort: Arc<Mutex<Option<String>>>,
}

/// Returns the first error happened, if any
pub fn check_abort(abort: &Mutex<Option<String>>) -> Result<(), Error> {
    let guard = abort.lock().unwrap();
    if let Some(err_msg) = &*guard {
        return Err(format_err!("{}", err_msg));
    }
    Ok(())
}

impl<I: Send> SendHandle<I> {
    /// Send data to the worker threads
    pub fn send(&self, input: I) -> Result<(), Error> {
        check_abort(&self.abort)?;
        match self.input.send(input) {
            Ok(()) => Ok(()),
            Err(_) => bail!("send failed - channel closed"),
        }
    }
}

/// A thread pool which run the supplied closure
///
/// The send command sends data to the worker threads. If one handler
/// returns an error, we mark the channel as failed and it is no
/// longer possible to send data.
///
/// When done, the 'complete()' method needs to be called to check for
/// outstanding errors.
pub struct ParallelHandler<'a, I> {
    handles: Vec<JoinHandle<()>>,
    name: String,
    input: Option<SendHandle<I>>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<I> Clone for SendHandle<I> {
    fn clone(&self) -> Self {
        Self {
            input: self.input.clone(),
            abort: Arc::clone(&self.abort),
        }
    }
}

impl<'a, I: Send + 'static> ParallelHandler<'a, I> {
    /// Create a new thread pool, each thread processing incoming data
    /// with 'handler_fn'.
    pub fn new<F>(name: &str, threads: usize, handler_fn: F) -> Self
        where F: Fn(I) -> Result<(), Error> + Send + Clone + 'a,
    {
        let mut handles = Vec::new();
        let (input_tx, input_rx) = bounded::<I>(threads);

        let abort = Arc::new(Mutex::new(None));

        for i in 0..threads {
            let input_rx = input_rx.clone();
            let abort = Arc::clone(&abort);

            // Erase the 'a lifetime bound. This is safe because we
            // join all thread in the drop handler.
            let handler_fn: Box<dyn Fn(I) -> Result<(), Error> + Send + 'a> =
                Box::new(handler_fn.clone());
            let handler_fn: Box<dyn Fn(I) -> Result<(), Error> + Send + 'static> =
                unsafe { std::mem::transmute(handler_fn) };

            handles.push(
                std::thread::Builder::new()
                    .name(format!("{} ({})", name, i))
                    .spawn(move || loop {
                        let data = match input_rx.recv() {
                            Ok(data) => data,
                            Err(_) => return,
                        };
                        match (handler_fn)(data) {
                            Ok(()) => (),
                            Err(err) => {
                                let mut guard = abort.lock().unwrap();
                                if guard.is_none() {
                                    *guard = Some(err.to_string());
                                }
                            }
                        }
                    })
                    .unwrap()
            );
        }
        Self {
            handles,
            name: name.to_string(),
            input: Some(SendHandle {
                input: input_tx,
                abort,
            }),
            _marker: std::marker::PhantomData,
        }
    }

    /// Returns a cloneable channel to send data to the worker threads
    pub fn channel(&self) -> SendHandle<I> {
        self.input.as_ref().unwrap().clone()
    }

    /// Send data to the worker threads
    pub fn send(&self, input: I) -> Result<(), Error> {
        self.input.as_ref().unwrap().send(input)?;
        Ok(())
    }

    /// Wait for worker threads to complete and check for errors
    pub fn complete(mut self) -> Result<(), Error> {
        let input = self.input.take().unwrap();
        let abort = Arc::clone(&input.abort);
        check_abort(&abort)?;
        drop(input);

        let msg_list = self.join_threads();

        // an error might be encountered while waiting for the join
        check_abort(&abort)?;

        if msg_list.is_empty() {
            return Ok(());
        }
        Err(format_err!("{}", msg_list.join("\n")))
    }

    fn join_threads(&mut self) -> Vec<String> {

        let mut msg_list = Vec::new();

        let mut i = 0;
        loop {
            let handle = match self.handles.pop() {
                Some(handle) => handle,
                None => break,
            };
            if let Err(panic) = handle.join() {
                match panic.downcast::<&str>() {
                    Ok(panic_msg) => msg_list.push(
                        format!("thread {} ({}) paniced: {}", self.name, i, panic_msg)
                    ),
                    Err(_) => msg_list.push(
                        format!("thread {} ({}) paniced", self.name, i)
                    ),
                }
            }
            i += 1;
        }
        msg_list
    }
}

// Note: We make sure that all threads will be joined
impl<'a, I> Drop for ParallelHandler<'a, I> {
    fn drop(&mut self) {
        drop(self.input.take());
        while let Some(handle) = self.handles.pop() {
            let _ = handle.join();
        }
    }
}
