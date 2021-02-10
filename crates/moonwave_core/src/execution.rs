use std::{
  sync::atomic::{AtomicUsize, Ordering},
  thread,
};

use async_task::Runnable;
use core_affinity::CoreId;
use flume::{Receiver, Sender};
use futures::{executor::block_on, future::join_all, Future};
use thread::Builder;

/// Describes a rough estimation for an tasks execution time.
pub enum EstimatedExecutionTime {
  /// Task will take a certain fraction of a frame
  FractionOfFrame(u8),
  /// Task has a very varying execution time.
  Varying,
  /// Task is very small and has no execution time that would matter in an balancing context.
  Unrelevant,
  /// Task has no specified execution time.
  Unspecified,
}

impl EstimatedExecutionTime {
  pub fn get_cost(&self) -> usize {
    match self {
      EstimatedExecutionTime::Unrelevant => 10,
      EstimatedExecutionTime::Unspecified => 100,
      EstimatedExecutionTime::Varying => 1000,
      EstimatedExecutionTime::FractionOfFrame(x) => 100000 / *x as usize,
    }
  }
}

pub struct LoadBalancedPool {
  scheduler: Vec<(ThreadedScheduler, AtomicUsize)>,
}

impl LoadBalancedPool {
  pub fn new(name: &str, cores: &[CoreId], collectable: bool) -> Self {
    Self {
      scheduler: cores
        .iter()
        .enumerate()
        .map(|(index, core)| {
          (
            ThreadedScheduler::new(format!("{} #{}", name, index + 1), *core, collectable),
            AtomicUsize::new(0),
          )
        })
        .collect::<Vec<_>>(),
    }
  }

  pub fn add_task<F: Future + Send + Sync + 'static>(
    &self,
    future: F,
    estimation: EstimatedExecutionTime,
  ) -> impl Future<Output = F::Output>
  where
    F::Output: Send + Sync + 'static,
  {
    // Get worker with smallest load
    let (mut lowest, mut lowest_index) = (0, 0);
    for (index, (_, load)) in self.scheduler.iter().enumerate() {
      let load = load.load(Ordering::Relaxed);
      if load < lowest {
        lowest = load;
        lowest_index = index;
      }
    }

    // Increment load
    self
      .scheduler
      .get(lowest_index)
      .unwrap()
      .1
      .fetch_add(estimation.get_cost(), Ordering::Relaxed);

    // Spawn task
    self.scheduler.get(lowest_index).unwrap().0.spawn(future)
  }

  pub fn collect(&self) -> impl Future {
    let futures = self
      .scheduler
      .iter()
      .map(|(scheduler, _)| scheduler.collect());
    join_all(futures)
  }

  pub fn run(&self) {
    for (scheduler, _) in &self.scheduler {
      scheduler.run();
    }
  }
}

pub struct Execution {
  background_workers: LoadBalancedPool,
  ecs_workers: LoadBalancedPool,
  graph_workers: LoadBalancedPool,
  main: Scheduler,
}

impl Execution {
  pub fn new(background_percentage: f32) -> Self {
    // Receive available cores.
    let core_ids = core_affinity::get_core_ids().unwrap();
    let background_workers_cores_len = (core_ids.len() as f32 * background_percentage) as usize;
    let background_workers_cores = &core_ids[..background_workers_cores_len];
    let workers_cores = &core_ids[background_workers_cores_len..];

    // Created load balanced pools.
    let background_workers =
      LoadBalancedPool::new("Background Worker", background_workers_cores, false);
    let ecs_workers = LoadBalancedPool::new("ECS Worker", workers_cores, true);
    let graph_workers = LoadBalancedPool::new("Graph Worker", workers_cores, true);

    Self {
      background_workers,
      ecs_workers,
      graph_workers,
      main: Scheduler::new(),
    }
  }

  pub fn add_background_task<F: Future + Send + Sync + 'static>(
    &self,
    future: F,
    estimation: EstimatedExecutionTime,
  ) -> impl Future<Output = F::Output>
  where
    F::Output: Send + Sync + 'static,
  {
    self.background_workers.add_task(future, estimation)
  }

  pub fn add_ecs_task<F: Future + Send + Sync + 'static>(
    &self,
    future: F,
    estimation: EstimatedExecutionTime,
  ) -> impl Future<Output = F::Output>
  where
    F::Output: Send + Sync + 'static,
  {
    self.ecs_workers.add_task(future, estimation)
  }

  pub fn add_graph_task<F: Future + Send + Sync + 'static>(
    &self,
    future: F,
    estimation: EstimatedExecutionTime,
  ) -> impl Future<Output = F::Output>
  where
    F::Output: Send + Sync + 'static,
  {
    self.graph_workers.add_task(future, estimation)
  }

  pub fn add_main_task<F: Future + Send + Sync + 'static>(
    &self,
    future: F,
  ) -> impl Future<Output = F::Output>
  where
    F::Output: Send + Sync + 'static,
  {
    self.main.spawn(future)
  }

  pub fn block_ecs(&self) {
    block_on(self.ecs_workers.collect());
  }

  pub fn block_graph(&self) {
    block_on(self.graph_workers.collect());
  }

  pub fn block_main(&self) {
    self.main.collect();
  }

  pub fn start(&self) {
    self.background_workers.run();
  }
}

struct Scheduler {
  sender: Sender<Runnable>,
  receiver: Receiver<Runnable>,
}

impl Scheduler {
  pub fn new() -> Self {
    let (sender, receiver): (Sender<Runnable>, Receiver<Runnable>) = flume::unbounded();
    Self { sender, receiver }
  }

  pub fn collect(&self) {
    for task in self.receiver.try_iter() {
      task.run();
    }
  }

  pub fn spawn<F: Future + Send + Sync + 'static>(
    &self,
    future: F,
  ) -> impl Future<Output = F::Output>
  where
    F::Output: Send + Sync + 'static,
  {
    let sender = self.sender.clone();
    let schedule = move |runnable| sender.send(runnable).unwrap();
    let (runnable, task) = async_task::spawn(future, schedule);
    runnable.schedule();
    task
  }
}

struct ThreadedScheduler {
  sender: Sender<Runnable>,
  sender_global: Sender<Runnable>,
}

impl ThreadedScheduler {
  pub fn new(name: String, core: CoreId, collectable: bool) -> Self {
    // Build task channel
    let (sender, receiver): (Sender<Runnable>, Receiver<Runnable>) = flume::unbounded();

    // Build singal channel
    let (sender_global, receiver_global): (Sender<Runnable>, Receiver<Runnable>) =
      flume::bounded(1);

    // Build thread
    Builder::new()
      .name(name.clone())
      .spawn(move || {
        // Prepare thread.
        core_affinity::set_for_current(core);
        optick::register_thread(name.as_str());

        // Wait for a message to collect all tasks
        for signal in receiver_global {
          if collectable {
            for task in receiver.try_iter() {
              task.run();
            }
          } else {
            for task in receiver.iter() {
              task.run();
            }
          };

          signal.run();
        }
      })
      .unwrap();

    Self {
      sender,
      sender_global,
    }
  }

  pub fn spawn<F: Future + Send + Sync + 'static>(
    &self,
    future: F,
  ) -> impl Future<Output = F::Output>
  where
    F::Output: Send + Sync + 'static,
  {
    let sender = self.sender.clone();
    let schedule = move |runnable| sender.send(runnable).unwrap();
    let (runnable, task) = async_task::spawn(future, schedule);
    runnable.schedule();
    task
  }

  pub fn run(&self) {
    let sender = self.sender_global.clone();
    let probe = async {};
    let schedule = move |runnable| sender.send(runnable).unwrap();
    let (runnable, _task) = async_task::spawn(probe, schedule);
    runnable.schedule();
  }

  pub fn collect(&self) -> impl Future {
    let sender = self.sender_global.clone();
    let probe = async {};
    let schedule = move |runnable| sender.send(runnable).unwrap();
    let (runnable, task) = async_task::spawn(probe, schedule);
    runnable.schedule();
    task
  }
}
