mod build_support;

fn main() {
    build_support::emit().expect("collect PCP build identity");
}
