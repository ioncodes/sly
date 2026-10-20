use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

struct State<Request, Response> {
    generation: u64,
    request: Option<(u64, Request)>,
    result: Option<Response>,
    shutdown: bool,
}

type Shared<Request, Response> = Arc<(Mutex<State<Request, Response>>, Condvar)>;

pub struct LatestJob<Request, Response> {
    shared: Shared<Request, Response>,
    pending: bool,
}

impl<Request: Send + 'static, Response: Send + 'static> LatestJob<Request, Response> {
    /// `job` can poll whether its request has been superseded.
    pub fn spawn(
        name: &str,
        job: impl Fn(Request, &dyn Fn() -> bool) -> Response + Send + 'static,
    ) -> io::Result<Self> {
        let shared: Shared<Request, Response> = Arc::new((
            Mutex::new(State {
                generation: 0,
                request: None,
                result: None,
                shutdown: false,
            }),
            Condvar::new(),
        ));

        let worker = shared.clone();

        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let (mutex, wake) = &*worker;

                loop {
                    let (generation, request) = {
                        let mut state = lock(mutex);

                        while state.request.is_none() && !state.shutdown {
                            state = wake.wait(state).unwrap_or_else(|error| error.into_inner());
                        }

                        if state.shutdown {
                            return;
                        }

                        state.request.take().expect("request checked above")
                    };

                    let cancelled = || {
                        let state = lock(mutex);
                        state.shutdown || state.generation != generation
                    };

                    let result = job(request, &cancelled);

                    let mut state = lock(mutex);

                    if state.shutdown {
                        return;
                    }

                    if state.generation == generation {
                        state.result = Some(result);
                    }
                }
            })?;

        Ok(Self {
            shared,
            pending: false,
        })
    }

    pub fn request(&mut self, request: Request) {
        let (mutex, wake) = &*self.shared;
        let mut state = lock(mutex);
        state.generation = state.generation.wrapping_add(1);
        state.request = Some((state.generation, request));
        state.result = None;
        self.pending = true;
        wake.notify_one();
    }

    pub fn cancel(&mut self) {
        let mut state = lock(&self.shared.0);
        state.generation = state.generation.wrapping_add(1);
        state.request = None;
        state.result = None;
        self.pending = false;
    }

    pub fn poll(&mut self) -> Option<Response> {
        let result = lock(&self.shared.0).result.take();

        if result.is_some() {
            self.pending = false;
        }

        result
    }
}

impl<Request, Response> Drop for LatestJob<Request, Response> {
    fn drop(&mut self) {
        let (mutex, wake) = &*self.shared;
        let mut state = lock(mutex);
        state.shutdown = true;
        state.request = None;
        state.result = None;
        wake.notify_one();
    }
}
