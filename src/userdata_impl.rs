use std::any::TypeId;
use std::cell::{Ref, RefCell, RefMut};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex, RwLock};

use crate::error::{Error, Result};
use crate::ffi;
use crate::lua::Lua;
use crate::types::{Callback, MaybeSend};
use crate::userdata::{
    AnyUserData, MetaMethod, UserData, UserDataCell, UserDataFields, UserDataMethods,
};
use crate::util::{check_stack, get_userdata, StackGuard};
use crate::value::{FromLua, FromLuaMulti, IntoLua, IntoLuaMulti, Value};

#[cfg(not(feature = "send"))]
use std::rc::Rc;

#[cfg(feature = "async")]
use {
    crate::types::AsyncCallback,
    futures_util::future::{self, TryFutureExt},
    std::future::Future,
};

pub(crate) struct StaticUserDataMethods<'lua, T: UserData + 'static> {
    pub(crate) methods: Vec<(String, Callback<'lua, 'static>)>,
    #[cfg(feature = "async")]
    pub(crate) async_methods: Vec<(String, AsyncCallback<'lua, 'static>)>,
    pub(crate) meta_methods: Vec<(String, Callback<'lua, 'static>)>,
    #[cfg(feature = "async")]
    pub(crate) async_meta_methods: Vec<(String, AsyncCallback<'lua, 'static>)>,
    _type: PhantomData<T>,
}

impl<'lua, T: UserData + 'static> Default for StaticUserDataMethods<'lua, T> {
    fn default() -> StaticUserDataMethods<'lua, T> {
        StaticUserDataMethods {
            methods: Vec::new(),
            #[cfg(feature = "async")]
            async_methods: Vec::new(),
            meta_methods: Vec::new(),
            #[cfg(feature = "async")]
            async_meta_methods: Vec::new(),
            _type: PhantomData,
        }
    }
}

impl<'lua, T: UserData + 'static> UserDataMethods<'lua, T> for StaticUserDataMethods<'lua, T> {
    fn add_method<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.methods
            .push((name.as_ref().into(), Self::box_method(method)));
    }

    fn add_method_mut<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.methods
            .push((name.as_ref().into(), Self::box_method_mut(method)));
    }

    #[cfg(feature = "async")]
    fn add_async_method<M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        T: Clone,
        M: Fn(&'lua Lua, T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        self.async_methods
            .push((name.as_ref().into(), Self::box_async_method(method)));
    }

    fn add_function<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.methods
            .push((name.as_ref().into(), Self::box_function(function)));
    }

    fn add_function_mut<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.methods
            .push((name.as_ref().into(), Self::box_function_mut(function)));
    }

    #[cfg(feature = "async")]
    fn add_async_function<F, A, FR, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        self.async_methods
            .push((name.as_ref().into(), Self::box_async_function(function)));
    }

    fn add_meta_method<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.meta_methods
            .push((name.as_ref().into(), Self::box_method(method)));
    }

    fn add_meta_method_mut<M, A, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.meta_methods
            .push((name.as_ref().into(), Self::box_method_mut(method)));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_method<M, A, MR, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        T: Clone,
        M: Fn(&'lua Lua, T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        self.async_meta_methods
            .push((name.as_ref().into(), Self::box_async_method(method)));
    }

    fn add_meta_function<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.meta_methods
            .push((name.as_ref().into(), Self::box_function(function)));
    }

    fn add_meta_function_mut<F, A, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        self.meta_methods
            .push((name.as_ref().into(), Self::box_function_mut(function)));
    }

    #[cfg(all(feature = "async", not(any(feature = "lua51", feature = "luau"))))]
    fn add_async_meta_function<F, A, FR, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        self.async_meta_methods
            .push((name.as_ref().into(), Self::box_async_function(function)));
    }

    // Below are internal methods used in generated code

    fn add_callback(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_callback(&mut self, name: String, callback: AsyncCallback<'lua, 'static>) {
        self.async_methods.push((name, callback));
    }

    fn add_meta_callback(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.meta_methods.push((name, callback));
    }

    #[cfg(feature = "async")]
    fn add_async_meta_callback(&mut self, meta: String, callback: AsyncCallback<'lua, 'static>) {
        self.async_meta_methods.push((meta, callback))
    }
}

impl<'lua, T: UserData + 'static> StaticUserDataMethods<'lua, T> {
    fn box_method<M, A, R>(method: M) -> Callback<'lua, 'static>
    where
        M: Fn(&'lua Lua, &T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        Box::new(move |lua, mut args| {
            if let Some(front) = args.pop_front() {
                let state = lua.state();
                let userdata = AnyUserData::from_lua(front, lua)?;
                unsafe {
                    let _sg = StackGuard::new(state);
                    check_stack(state, 2)?;

                    let type_id = lua.push_userdata_ref(&userdata.0)?;
                    match type_id {
                        Some(id) if id == TypeId::of::<T>() => {
                            let ud = get_userdata_ref::<T>(state)?;
                            method(lua, &ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        #[cfg(not(feature = "send"))]
                        Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => {
                            let ud = get_userdata_ref::<Rc<RefCell<T>>>(state)?;
                            let ud = ud.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
                            method(lua, &ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => {
                            let ud = get_userdata_ref::<Arc<Mutex<T>>>(state)?;
                            let ud = ud.try_lock().map_err(|_| Error::UserDataBorrowError)?;
                            method(lua, &ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => {
                            let ud = get_userdata_ref::<Arc<parking_lot::Mutex<T>>>(state)?;
                            let ud = ud.try_lock().ok_or(Error::UserDataBorrowError)?;
                            method(lua, &ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => {
                            let ud = get_userdata_ref::<Arc<RwLock<T>>>(state)?;
                            let ud = ud.try_read().map_err(|_| Error::UserDataBorrowError)?;
                            method(lua, &ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => {
                            let ud = get_userdata_ref::<Arc<parking_lot::RwLock<T>>>(state)?;
                            let ud = ud.try_read().ok_or(Error::UserDataBorrowError)?;
                            method(lua, &ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        _ => Err(Error::UserDataTypeMismatch),
                    }
                }
            } else {
                Err(Error::FromLuaConversionError {
                    from: "missing argument",
                    to: "userdata",
                    message: None,
                })
            }
        })
    }

    fn box_method_mut<M, A, R>(method: M) -> Callback<'lua, 'static>
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let method = RefCell::new(method);
        Box::new(move |lua, mut args| {
            if let Some(front) = args.pop_front() {
                let state = lua.state();
                let userdata = AnyUserData::from_lua(front, lua)?;
                let mut method = method
                    .try_borrow_mut()
                    .map_err(|_| Error::RecursiveMutCallback)?;
                unsafe {
                    let _sg = StackGuard::new(state);
                    check_stack(state, 2)?;

                    let type_id = lua.push_userdata_ref(&userdata.0)?;
                    match type_id {
                        Some(id) if id == TypeId::of::<T>() => {
                            let mut ud = get_userdata_mut::<T>(state)?;
                            method(lua, &mut ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        #[cfg(not(feature = "send"))]
                        Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => {
                            let ud = get_userdata_mut::<Rc<RefCell<T>>>(state)?;
                            let mut ud = ud
                                .try_borrow_mut()
                                .map_err(|_| Error::UserDataBorrowMutError)?;
                            method(lua, &mut ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => {
                            let ud = get_userdata_mut::<Arc<Mutex<T>>>(state)?;
                            let mut ud =
                                ud.try_lock().map_err(|_| Error::UserDataBorrowMutError)?;
                            method(lua, &mut ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => {
                            let ud = get_userdata_mut::<Arc<parking_lot::Mutex<T>>>(state)?;
                            let mut ud = ud.try_lock().ok_or(Error::UserDataBorrowMutError)?;
                            method(lua, &mut ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => {
                            let ud = get_userdata_mut::<Arc<RwLock<T>>>(state)?;
                            let mut ud =
                                ud.try_write().map_err(|_| Error::UserDataBorrowMutError)?;
                            method(lua, &mut ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        #[cfg(feature = "parking_lot")]
                        Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => {
                            let ud = get_userdata_mut::<Arc<parking_lot::RwLock<T>>>(state)?;
                            let mut ud = ud.try_write().ok_or(Error::UserDataBorrowMutError)?;
                            method(lua, &mut ud, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
                        }
                        _ => Err(Error::UserDataTypeMismatch),
                    }
                }
            } else {
                Err(Error::FromLuaConversionError {
                    from: "missing argument",
                    to: "userdata",
                    message: None,
                })
            }
        })
    }

    #[cfg(feature = "async")]
    fn box_async_method<M, A, MR, R>(method: M) -> AsyncCallback<'lua, 'static>
    where
        T: Clone,
        M: Fn(&'lua Lua, T, A) -> MR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        MR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        Box::new(move |lua, mut args| {
            let fut_res = || {
                if let Some(front) = args.pop_front() {
                    let state = lua.state();
                    let userdata = AnyUserData::from_lua(front, lua)?;
                    unsafe {
                        let _sg = StackGuard::new(state);
                        check_stack(state, 2)?;

                        let type_id = lua.push_userdata_ref(&userdata.0)?;
                        match type_id {
                            Some(id) if id == TypeId::of::<T>() => {
                                let ud = get_userdata_ref::<T>(state)?;
                                Ok(method(lua, ud.clone(), A::from_lua_multi(args, lua)?))
                            }
                            #[cfg(not(feature = "send"))]
                            Some(id) if id == TypeId::of::<Rc<RefCell<T>>>() => {
                                let ud = get_userdata_ref::<Rc<RefCell<T>>>(state)?;
                                let ud = ud.try_borrow().map_err(|_| Error::UserDataBorrowError)?;
                                Ok(method(lua, ud.clone(), A::from_lua_multi(args, lua)?))
                            }
                            Some(id) if id == TypeId::of::<Arc<Mutex<T>>>() => {
                                let ud = get_userdata_ref::<Arc<Mutex<T>>>(state)?;
                                let ud = ud.try_lock().map_err(|_| Error::UserDataBorrowError)?;
                                Ok(method(lua, ud.clone(), A::from_lua_multi(args, lua)?))
                            }
                            #[cfg(feature = "parking_lot")]
                            Some(id) if id == TypeId::of::<Arc<parking_lot::Mutex<T>>>() => {
                                let ud = get_userdata_ref::<Arc<parking_lot::Mutex<T>>>(state)?;
                                let ud = ud.try_lock().ok_or(Error::UserDataBorrowError)?;
                                Ok(method(lua, ud.clone(), A::from_lua_multi(args, lua)?))
                            }
                            Some(id) if id == TypeId::of::<Arc<RwLock<T>>>() => {
                                let ud = get_userdata_ref::<Arc<RwLock<T>>>(state)?;
                                let ud = ud.try_read().map_err(|_| Error::UserDataBorrowError)?;
                                Ok(method(lua, ud.clone(), A::from_lua_multi(args, lua)?))
                            }
                            #[cfg(feature = "parking_lot")]
                            Some(id) if id == TypeId::of::<Arc<parking_lot::RwLock<T>>>() => {
                                let ud = get_userdata_ref::<Arc<parking_lot::RwLock<T>>>(state)?;
                                let ud = ud.try_read().ok_or(Error::UserDataBorrowError)?;
                                Ok(method(lua, ud.clone(), A::from_lua_multi(args, lua)?))
                            }
                            _ => Err(Error::UserDataTypeMismatch),
                        }
                    }
                } else {
                    Err(Error::FromLuaConversionError {
                        from: "missing argument",
                        to: "userdata",
                        message: None,
                    })
                }
            };
            match fut_res() {
                Ok(fut) => {
                    Box::pin(fut.and_then(move |ret| future::ready(ret.into_lua_multi(lua))))
                }
                Err(e) => Box::pin(future::err(e)),
            }
        })
    }

    fn box_function<F, A, R>(function: F) -> Callback<'lua, 'static>
    where
        F: Fn(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        Box::new(move |lua, args| function(lua, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua))
    }

    fn box_function_mut<F, A, R>(function: F) -> Callback<'lua, 'static>
    where
        F: FnMut(&'lua Lua, A) -> Result<R> + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        R: IntoLuaMulti<'lua>,
    {
        let function = RefCell::new(function);
        Box::new(move |lua, args| {
            let function = &mut *function
                .try_borrow_mut()
                .map_err(|_| Error::RecursiveMutCallback)?;
            function(lua, A::from_lua_multi(args, lua)?)?.into_lua_multi(lua)
        })
    }

    #[cfg(feature = "async")]
    fn box_async_function<F, A, FR, R>(function: F) -> AsyncCallback<'lua, 'static>
    where
        F: Fn(&'lua Lua, A) -> FR + MaybeSend + 'static,
        A: FromLuaMulti<'lua>,
        FR: Future<Output = Result<R>> + 'lua,
        R: IntoLuaMulti<'lua>,
    {
        Box::new(move |lua, args| {
            let args = match A::from_lua_multi(args, lua) {
                Ok(args) => args,
                Err(e) => return Box::pin(future::err(e)),
            };
            Box::pin(
                function(lua, args).and_then(move |ret| future::ready(ret.into_lua_multi(lua))),
            )
        })
    }
}

pub(crate) struct StaticUserDataFields<'lua, T: UserData + 'static> {
    pub(crate) field_getters: Vec<(String, Callback<'lua, 'static>)>,
    pub(crate) field_setters: Vec<(String, Callback<'lua, 'static>)>,
    #[allow(clippy::type_complexity)]
    pub(crate) meta_fields: Vec<(
        String,
        Box<dyn Fn(&'lua Lua) -> Result<Value<'lua>> + 'static>,
    )>,
    _type: PhantomData<T>,
}

impl<'lua, T: UserData + 'static> Default for StaticUserDataFields<'lua, T> {
    fn default() -> StaticUserDataFields<'lua, T> {
        StaticUserDataFields {
            field_getters: Vec::new(),
            field_setters: Vec::new(),
            meta_fields: Vec::new(),
            _type: PhantomData,
        }
    }
}

impl<'lua, T: UserData + 'static> UserDataFields<'lua, T> for StaticUserDataFields<'lua, T> {
    fn add_field_method_get<M, R>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: Fn(&'lua Lua, &T) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let method = StaticUserDataMethods::box_method(move |lua, data, ()| method(lua, data));
        self.field_getters.push((name.as_ref().into(), method));
    }

    fn add_field_method_set<M, A>(&mut self, name: impl AsRef<str>, method: M)
    where
        M: FnMut(&'lua Lua, &mut T, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua<'lua>,
    {
        let method = StaticUserDataMethods::box_method_mut(method);
        self.field_setters.push((name.as_ref().into(), method));
    }

    fn add_field_function_get<F, R>(&mut self, name: impl AsRef<str>, function: F)
    where
        F: Fn(&'lua Lua, AnyUserData<'lua>) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let func = StaticUserDataMethods::<T>::box_function(function);
        self.field_getters.push((name.as_ref().into(), func));
    }

    fn add_field_function_set<F, A>(&mut self, name: impl AsRef<str>, mut function: F)
    where
        F: FnMut(&'lua Lua, AnyUserData<'lua>, A) -> Result<()> + MaybeSend + 'static,
        A: FromLua<'lua>,
    {
        let func = StaticUserDataMethods::<T>::box_function_mut(move |lua, (data, val)| {
            function(lua, data, val)
        });
        self.field_setters.push((name.as_ref().into(), func));
    }

    fn add_meta_field_with<F, R>(&mut self, name: impl AsRef<str>, f: F)
    where
        F: Fn(&'lua Lua) -> Result<R> + MaybeSend + 'static,
        R: IntoLua<'lua>,
    {
        let name = name.as_ref().to_string();
        self.meta_fields.push((
            name.clone(),
            Box::new(move |lua| {
                let value = f(lua)?.into_lua(lua)?;
                if name == MetaMethod::Index || name == MetaMethod::NewIndex {
                    match value {
                        Value::Nil | Value::Table(_) | Value::Function(_) => {}
                        _ => {
                            return Err(Error::MetaMethodTypeError {
                                method: name.to_string(),
                                type_name: value.type_name(),
                                message: Some("expected nil, table or function".to_string()),
                            })
                        }
                    }
                }
                Ok(value)
            }),
        ));
    }

    // Below are internal methods

    fn add_field_getter(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.field_getters.push((name, callback));
    }

    fn add_field_setter(&mut self, name: String, callback: Callback<'lua, 'static>) {
        self.field_setters.push((name, callback));
    }
}

#[inline]
unsafe fn get_userdata_ref<'a, T>(state: *mut ffi::lua_State) -> Result<Ref<'a, T>> {
    (*get_userdata::<UserDataCell<T>>(state, -1)).try_borrow()
}

#[inline]
unsafe fn get_userdata_mut<'a, T>(state: *mut ffi::lua_State) -> Result<RefMut<'a, T>> {
    (*get_userdata::<UserDataCell<T>>(state, -1)).try_borrow_mut()
}

macro_rules! lua_userdata_impl {
    ($type:ty) => {
        impl<T: UserData + 'static> UserData for $type {
            fn add_fields<'lua, F: UserDataFields<'lua, Self>>(fields: &mut F) {
                let mut orig_fields = StaticUserDataFields::default();
                T::add_fields(&mut orig_fields);
                for (name, callback) in orig_fields.field_getters {
                    fields.add_field_getter(name, callback);
                }
                for (name, callback) in orig_fields.field_setters {
                    fields.add_field_setter(name, callback);
                }
            }

            fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
                let mut orig_methods = StaticUserDataMethods::default();
                T::add_methods(&mut orig_methods);
                for (name, callback) in orig_methods.methods {
                    methods.add_callback(name, callback);
                }
                #[cfg(feature = "async")]
                for (name, callback) in orig_methods.async_methods {
                    methods.add_async_callback(name, callback);
                }
                for (meta, callback) in orig_methods.meta_methods {
                    methods.add_meta_callback(meta, callback);
                }
                #[cfg(feature = "async")]
                for (meta, callback) in orig_methods.async_meta_methods {
                    methods.add_async_meta_callback(meta, callback);
                }
            }
        }
    };
}

#[cfg(not(feature = "send"))]
lua_userdata_impl!(Rc<RefCell<T>>);
lua_userdata_impl!(Arc<Mutex<T>>);
lua_userdata_impl!(Arc<RwLock<T>>);
#[cfg(feature = "parking_lot")]
lua_userdata_impl!(Arc<parking_lot::Mutex<T>>);
#[cfg(feature = "parking_lot")]
lua_userdata_impl!(Arc<parking_lot::RwLock<T>>);

// A special proxy object for UserData
pub(crate) struct UserDataProxy<T>(pub(crate) PhantomData<T>);

lua_userdata_impl!(UserDataProxy<T>);
