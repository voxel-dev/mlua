use mlua::{Function, Lua, Result, String};

#[test]
fn test_function() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function concat(arg1, arg2)
            return arg1 .. arg2
        end
    "#,
    )
    .exec()?;

    let concat = globals.get::<_, Function>("concat")?;
    assert_eq!(concat.call::<_, String>(("foo", "bar"))?, "foobar");

    Ok(())
}

#[test]
fn test_bind() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function concat(...)
            local res = ""
            for _, s in pairs({...}) do
                res = res..s
            end
            return res
        end
    "#,
    )
    .exec()?;

    let mut concat = globals.get::<_, Function>("concat")?;
    concat = concat.bind("foo")?;
    concat = concat.bind("bar")?;
    concat = concat.bind(("baz", "baf"))?;
    assert_eq!(concat.call::<_, String>(())?, "foobarbazbaf");
    assert_eq!(
        concat.call::<_, String>(("hi", "wut"))?,
        "foobarbazbafhiwut"
    );

    let mut concat2 = globals.get::<_, Function>("concat")?;
    concat2 = concat2.bind(())?;
    assert_eq!(concat2.call::<_, String>(())?, "");
    assert_eq!(concat2.call::<_, String>(("ab", "cd"))?, "abcd");

    Ok(())
}

#[test]
fn test_rust_function() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function lua_function()
            return rust_function()
        end

        -- Test to make sure chunk return is ignored
        return 1
    "#,
    )
    .exec()?;

    let lua_function = globals.get::<_, Function>("lua_function")?;
    let rust_function = lua.create_function(|_, ()| Ok("hello"))?;

    globals.set("rust_function", rust_function)?;
    assert_eq!(lua_function.call::<_, String>(())?, "hello");

    Ok(())
}

#[test]
fn test_c_function() -> Result<()> {
    let lua = Lua::new();

    unsafe extern "C" fn c_function(state: *mut mlua::lua_State) -> std::os::raw::c_int {
        let lua = Lua::init_from_ptr(state);
        lua.globals().set("c_function", true).unwrap();
        0
    }

    let func = unsafe { lua.create_c_function(c_function)? };
    func.call(())?;
    assert_eq!(lua.globals().get::<_, bool>("c_function")?, true);

    Ok(())
}

#[cfg(not(feature = "luau"))]
#[test]
fn test_dump() -> Result<()> {
    let lua = unsafe { Lua::unsafe_new() };

    let concat_lua = lua
        .load(r#"function(arg1, arg2) return arg1 .. arg2 end"#)
        .eval::<Function>()?;
    let concat = lua.load(&concat_lua.dump(false)).into_function()?;

    assert_eq!(concat.call::<_, String>(("foo", "bar"))?, "foobar");

    Ok(())
}

#[test]
fn test_function_info() -> Result<()> {
    let lua = Lua::new();

    let globals = lua.globals();
    lua.load(
        r#"
        function function1()
            return function() end
        end
    "#,
    )
    .set_name("source1")
    .exec()?;

    let function1 = globals.get::<_, Function>("function1")?;
    let function2 = function1.call::<_, Function>(())?;
    let function3 = lua.create_function(|_, ()| Ok(()))?;

    let function1_info = function1.info();
    #[cfg(feature = "luau")]
    assert_eq!(function1_info.name, Some(b"function1".to_vec()));
    assert_eq!(function1_info.source, Some(b"source1".to_vec()));
    assert_eq!(function1_info.line_defined, 2);
    #[cfg(not(feature = "luau"))]
    assert_eq!(function1_info.last_line_defined, 4);
    assert_eq!(function1_info.what, Some(b"Lua".to_vec()));

    let function2_info = function2.info();
    assert_eq!(function2_info.name, None);
    assert_eq!(function2_info.source, Some(b"source1".to_vec()));
    assert_eq!(function2_info.line_defined, 3);
    #[cfg(not(feature = "luau"))]
    assert_eq!(function2_info.last_line_defined, 3);
    assert_eq!(function2_info.what, Some(b"Lua".to_vec()));

    let function3_info = function3.info();
    assert_eq!(function3_info.name, None);
    assert_eq!(function3_info.source, Some(b"=[C]".to_vec()));
    assert_eq!(function3_info.line_defined, -1);
    #[cfg(not(feature = "luau"))]
    assert_eq!(function3_info.last_line_defined, -1);
    assert_eq!(function3_info.what, Some(b"C".to_vec()));

    let print_info = globals.get::<_, Function>("print")?.info();
    #[cfg(feature = "luau")]
    assert_eq!(print_info.name, Some(b"print".to_vec()));
    assert_eq!(print_info.source, Some(b"=[C]".to_vec()));
    assert_eq!(print_info.what, Some(b"C".to_vec()));
    assert_eq!(print_info.line_defined, -1);

    Ok(())
}

#[cfg(feature = "unstable")]
#[test]
fn test_function_wrap() -> Result<()> {
    use mlua::Error;

    let lua = Lua::new();

    lua.globals()
        .set("f", Function::wrap(|_, s: String| Ok(s)))?;
    lua.load(r#"assert(f("hello") == "hello")"#).exec().unwrap();

    let mut _i = false;
    lua.globals().set(
        "f",
        Function::wrap_mut(move |lua, ()| {
            _i = true;
            lua.globals().get::<_, Function>("f")?.call::<_, ()>(())
        }),
    )?;
    match lua.globals().get::<_, Function>("f")?.call::<_, ()>(()) {
        Err(Error::CallbackError { ref cause, .. }) => match *cause.as_ref() {
            Error::CallbackError { ref cause, .. } => match *cause.as_ref() {
                Error::RecursiveMutCallback { .. } => {}
                ref other => panic!("incorrect result: {other:?}"),
            },
            ref other => panic!("incorrect result: {other:?}"),
        },
        other => panic!("incorrect result: {other:?}"),
    };

    Ok(())
}
