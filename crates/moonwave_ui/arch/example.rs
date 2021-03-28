struct App;

#[component]
impl App {
  #[render]
  fn render(&self, #[layout] layout: Layout) {
    render! {
        HStack {
            MyComponent
                .text("hello world")
                .number(12),
            MyComponent
                .text("hello friend")
                .number(42),
        }
    }
  }
}

struct MyComponent {
  text: String,
  number: usize,
}

#[component]
impl MyComponent {
  pub fn new() -> Self {
    Self
  }
  pub fn text(self, str: &str) -> Self {
    self
  }
  pub fn number(self, num: usize) -> Self {
    self
  }

  #[render]
  fn render(&self, #[layout] layout: &Layout) {
    let (width, height) = layout.get_max_available();
    let x = self.number * 2;
    render! {
        Text(format!("Txt: {}  Number*2: {} Width: {}", self.text, x, width)),
    }
  }
}

///
struct MyComponentStorage {
  txt: Option<TextComponent>,
}

impl Component for MyComponent {
  type Storage = MyComponentStorage;
  type UpdateFlags = MyComponentUpdateFlags;

  fn create(&self, storage: &mut Storage, alloc: &Allocator) {
    let x = self.number * 2;
    storage.txt = alloc.alloc(Text::new(format!("Txt: {}  Number*2: {}", self.text, x)));
  }

  fn mount(&self, storage: &Storage, parent: &mut ComponentHost) {
    parent.mount_child(storage.txt);
  }

  fn unmount(&self, storage: &Storage, parent: &mut ComponentHost) {
    parent.unmount_child(storage.txt);
  }

  fn update(&mut self, storage: &mut Storage, flag: MyComponentUpdateFlags) {
    let mut dirty_block_1 = false;

    match flag {
      MyComponentUpdateFlags::PropText(txt) => {
        self.text = txt;
        dirty_block_1 = true;
      }
      MyComponentUpdateFlags::PropNumber(num) => {
        self.number = num;
        dirty_block_1 = true;
      }
    }

    if (dirty_block_1) {
      let x = self.number * 2;
      let param_1 = format!("Txt: {}  Number*2: {}, Width: {}", self.text, x);
      storage
        .txt
        .push_update(TextUpdateFlags::from_method__new(param_1));
    }
  }
}
