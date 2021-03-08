// Case 1: Simplest
#[actor]
struct MyActor {
    foo: usize,
}

#[actor]
impl MyActor {
    pub fn new() -> Self {
        Self {
            foo: 12,
        }
    }
}


// Case 2: Dynamic sized children 
#[actor]
struct MyActor {
    children: Vec<ActorRef<MyOtherActor>>,
}


#[actor]
impl MyActor {
    pub fn new() -> Self {
        Self {
            children: Vec::new(),
        }
    }
    
    #[actor_spawn]
    pub fn on_spawn(&mut self) {
        self.children.push(self.spawn_actor(MyOtherActor::new(0)));
        self.children.push(self.spawn_actor(MyOtherActor::new(1)));
        self.children.push(self.spawn_actor(MyOtherActor::new(2)));
    }
}

// Case 3: Dynamic sized children with component
#[actor]
struct MyActor {
    offset: usize,
    children: Vec<ActorRef<MyOtherActor>>,
}


#[actor]
impl MyActor {
    pub fn new(offset_x: usize) -> Self {
        Self {
            children: Vec::new(),
        }
    }
    
    #[actor_spawn]
    pub fn on_spawn(&mut self) {
        self.add_component(Position::new(self.offset_x, 0.0, 0.0));
        
        self.children.push(self.spawn_actor(MyOtherActor::new(0)));
        self.children.push(self.spawn_actor(MyOtherActor::new(1)));
        self.children.push(self.spawn_actor(MyOtherActor::new(2)));
    }
    
    #[actor_tick(real)]
    pub fn tick(&self, pos: &mut Position) {
        pos.x += 1.0;
    }
}


// Case 4: Dynamic sized children with component and multi threaded
#[actor]
struct MyActor {
    children: Vec<ActorRef<MyOtherActor>>,
}

#[actor]
impl MyActor {
    pub fn new(offset_x: 0) -> Self {
        Self {
            children: Vec::new(),
        }
    }
    
    #[actor_spawn(background)]
    pub fn on_spawn(self: ActorCommandBuffer<self>) {
        self.add_component(Position::new(self.offset_x))
        
        let actors = 0..3.par_iter().map(|i| self.spawn_actor(MyOtherActor::new(i)));
        
        self.execute_mut(|inner| {
            inner.children.extend(actors);
        });
    }
    
    
    #[actor_tick(real)]
    pub fn tick(&self, pos: &mut Position) {
        pos.x += 1.0;
    }
}


// Case 5: Known child
#[actor]
struct MyActor {
  known: ActorRef<MyOtherActor>,
}

#[actor]
impl MyActor {
  pub fn new() -> ActorBuilder<Self> {
    let builder = ActorBuilder::new();
  }
}