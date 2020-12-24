#[macro_export(local_inner_macros)]
macro_rules! init_treemap {
    ($inst:ty) => {
        pub type GlobalInsnMap = TreeInsnMap;

        struct TreeNode {
            left: Option<*mut TreeNode>,
            right: Option<*mut TreeNode>,
            level: usize,
            value: Option<Box<dyn Decoder>>,
        }

        impl TreeNode {
            fn new(level: usize) -> TreeNode {
                TreeNode {
                    left: None,
                    right: None,
                    level: level,
                    value: None,
                }
            }

            fn max_level() -> usize { std::mem::size_of::<$inst>() << 3 }

            fn insert(&mut self, value: Box<dyn Decoder>) -> Option<&Box<dyn Decoder>> {
                if self.level == Self::max_level() {
                    if let Some(ref v) = self.value {
                        Some(v)
                    } else {
                        self.value = Some(value);
                        None
                    }
                } else {
                    let node = if value.code() & ((1 as $inst) << self.level as $inst) == 0 {
                        self.left.get_or_insert(Box::into_raw(Box::new(Self::new(self.level + 1))))
                    } else {
                        self.right.get_or_insert(Box::into_raw(Box::new(Self::new(self.level + 1))))
                    };
                    unsafe {
                        (*node).as_mut().unwrap().insert(value)
                    }
                }
            }

            fn get_node(node: *mut Self) -> *mut Self {
                let n = unsafe {
                    node.as_mut().unwrap()
                };
                if n.left.is_some() && n.right.is_none() {
                    Self::get_node(n.left.unwrap())
                } else if n.right.is_some() && n.left.is_none() {
                    Self::get_node(n.right.unwrap())
                } else {
                    node
                }
            }

            fn compress(&mut self) {
                let path_compress = |node: &mut Option<*mut Self>| {
                    if let Some(ref n) = node {
                        let new_node = Self::get_node(*n);
                        if *n != new_node {
                            *node = Some(new_node)
                        }
                    }
                };
                if self.level == Self::max_level() {
                    return;
                } else {
                    path_compress(&mut self.left);
                    path_compress(&mut self.right);
                    self.left.iter_mut().for_each(|n| { unsafe { n.as_mut().unwrap().compress() } });
                    self.right.iter_mut().for_each(|n| { unsafe { n.as_mut().unwrap().compress() } });
                }
            }

            fn get(&self, key: &$inst) -> Option<&Box<dyn Decoder>> {
                if self.level ==  Self::max_level() {
                    if let Some(ref v) = self.value {
                        if v.mask() & *key == v.code() {
                            return Some(v)
                        } else {
                            return None
                        }
                    }
                    __terminus_insn_unreachable!()
                } else {
                    if *key & ((1 as $inst) << self.level as $inst) == 0 {
                        if let Some(n) = self.left {
                            if let Some(v) = unsafe { n.as_ref().unwrap().get(key) } {
                                return Some(v);
                            }
                        }
                        if let Some(n) = self.right {
                            if let Some(v) = unsafe { n.as_ref().unwrap().get(key) } {
                                return Some(v);
                            }
                        }
                    } else {
                        if let Some(n) = self.right {
                            if let Some(v) = unsafe { n.as_ref().unwrap().get(key) } {
                                return Some(v);
                            }
                        }
                        if let Some(n) = self.left {
                            if let Some(v) = unsafe { n.as_ref().unwrap().get(key) } {
                                return Some(v);
                            }
                        }
                    }
                }
                None
            }
        }

        pub struct TreeInsnMap(TreeNode);

        impl TreeInsnMap {
            pub fn new() -> TreeInsnMap {
                TreeInsnMap(TreeNode::new(0))
            }
        }

        impl InsnMap for TreeInsnMap {
            fn registery<T: 'static + Decoder>(&mut self, decoder: T) {
                let name = decoder.name();
                let code = decoder.code();
                let mask = decoder.mask();
                if let Some(v) = self.0.insert(Box::new(decoder)) {
                    __terminus_insn_panic!(__terminus_insn_format!("inst {}(code = {:#x}; mask = {:#x}) is duplicated with inst {}(code = {:#x}; mask = {:#x})!", name, code, mask,v.name(), v.code(), v.mask()))
                }
            }

            fn decode(&self, ir: &$inst) -> Result<&Instruction, Error> {
                if let Some(decoder) = self.0.get(ir) {
                    Ok(decoder.decode())
                } else {
                    Err(Error::Illegal(*ir))
                }
            }
            fn lock(&mut self) {
                self.0.compress();
            }
        }

        //immutable after 'lock'
        unsafe impl Sync for TreeInsnMap {}

        //immutable after 'lock'
        unsafe impl Send for TreeInsnMap {}
    };
}