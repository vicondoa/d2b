fn main() {
    let parse = rnix::Root::parse("with lib; { apiVersion = 1; }");
    println!("{:#?}", parse.syntax());
}
