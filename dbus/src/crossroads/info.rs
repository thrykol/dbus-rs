use crate::strings::{Path as PathName, Interface as IfaceName, Member as MemberName, Signature};
use crate::Message;
use std::borrow::Cow;
use std::collections::HashMap;
use std::any::Any;
use std::mem;
use crate::arg::{Arg, Append, AppendAll, ReadAll, ArgAll, Get, TypeMismatchError, IterAppend};
use std::marker::PhantomData;
use super::MethodErr;
use super::handlers::{Handlers, MakeHandler, DebugMethod, DebugProp, Par, ParInfo, Mut, MutCtx};
use super::crossroads::{Crossroads, PathData};

fn build_argvec<A: ArgAll>(a: A::strs) -> Vec<Argument<'static>> {
    let mut v = vec!();
    A::strs_sig(a, |name, sig| {
        v.push(Argument { name: name.into(), sig, anns: Default::default() })
    });
    v
}

pub (super) type Annotations = HashMap<String, String>;

#[derive(Debug, Clone)]
pub struct Argument<'a> {
    name: Cow<'a, str>,
    sig: Signature<'a>,
    anns: Annotations,
}

#[derive(Debug)]
pub struct IfaceInfo<'a, H: Handlers> {
    pub (crate) name: IfaceName<'a>,
    pub (crate) methods: Vec<MethodInfo<'a, H>>,
    pub (crate) props: Vec<PropInfo<'a, H>>,
    pub (crate) signals: Vec<SignalInfo<'a>>,
    pub (super) anns: Annotations,
}

#[derive(Debug)]
pub struct MethodInfo<'a, H: Handlers> {
    name: MemberName<'a>,
    handler: DebugMethod<H>,
    i_args: Vec<Argument<'a>>,
    o_args: Vec<Argument<'a>>,
    pub (super) anns: Annotations,
}

impl<'a, H: Handlers> MethodInfo<'a, H> {
    pub fn name(&self) -> &MemberName<'a> { &self.name }
    pub fn handler(&self) -> &H::Method { &self.handler.0 }
    pub fn handler_mut(&mut self) -> &mut H::Method { &mut self.handler.0 }
}

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Debug)]
/// Enumerates the different signaling behaviors a Property can have
/// to being changed.
pub enum EmitsChangedSignal {
    /// The Property emits a signal that includes the new value.
    True,
    /// The Property emits a signal that does not include the new value.
    Invalidates,
    /// The Property cannot be changed.
    Const,
    /// The Property does not emit a signal when changed.
    False,
}

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Debug)]
/// The possible access characteristics a Property can have.
pub enum Access {
    /// The Property can only be read (Get).
    Read,
    /// The Property can be read or written.
    ReadWrite,
    /// The Property can only be written (Set).
    Write,
}

#[derive(Debug)]
pub struct PropInfo<'a, H: Handlers> {
    pub (crate) name: MemberName<'a>,
    pub (crate) handlers: DebugProp<H>,
    anns: Annotations,
    sig: Signature<'a>,
    emits: EmitsChangedSignal,
    auto_emit: bool,
    rw: Access,
}

#[derive(Debug)]
pub struct SignalInfo<'a> {
    name: MemberName<'a>,
    args: Vec<Argument<'a>>,
    pub (super) anns: Annotations,
}

#[derive(Debug, Clone, Copy)]
enum MetSigProp { Method, Signal, Prop }

#[derive(Debug)]
pub struct IfaceInfoBuilder<'a, I: 'static, H: Handlers> {
    cr: Option<&'a mut Crossroads<H>>,
    info: IfaceInfo<'static, H>,
    last: Option<MetSigProp>,
    _dummy: PhantomData<*const I>,
}

impl<'a, I, H: Handlers> IfaceInfoBuilder<'a, I, H> {
    pub fn new(cr: Option<&'a mut Crossroads<H>>, name: IfaceName<'static>) -> Self {
        IfaceInfoBuilder { cr, _dummy: PhantomData, info: IfaceInfo::new_empty(name), last: None }
    }

    pub fn signal<A: ArgAll, N: Into<MemberName<'static>>>(mut self, name: N, args: A::strs) -> Self {
        let s = SignalInfo { name: name.into(), args: build_argvec::<A>(args), anns: Default::default() };
        self.info.signals.push(s);
        self.last = Some(MetSigProp::Signal);
        self
    }

    /// Annotates the last added method, signal or property, or the interface itself if nothing is added.
    pub fn annotate<N: Into<String>, V: Into<String>>(mut self, name: N, value: V) -> Self {
         let x: &mut Annotations = match self.last {
             None => &mut self.info.anns,
             Some(MetSigProp::Method) => &mut self.info.methods.last_mut().unwrap().anns,
             Some(MetSigProp::Signal) => &mut self.info.signals.last_mut().unwrap().anns,
             Some(MetSigProp::Prop) => &mut self.info.props.last_mut().unwrap().anns,
         };
         x.insert(name.into(), value.into());
         self
    }

    /// Adds a deprecated annotation to the last added method/signal/property.
    pub fn deprecated(self) -> Self { self.annotate("org.freedesktop.DBus.Deprecated", "true") }
}

impl<'a, I: 'static, H: Handlers> Drop for IfaceInfoBuilder<'a, I, H> {
    fn drop(&mut self) {
        if let Some(ref mut cr) = self.cr {
            let info = IfaceInfo::new_empty(self.info.name.clone()); // workaround for not being able to consume self.info
            cr.register_custom::<I>(mem::replace(&mut self.info, info));
        }
    }
}

impl<'a, I: 'static, H: Handlers> IfaceInfoBuilder<'a, I, H> {
    pub fn method<IA: ReadAll + ArgAll, OA: AppendAll + ArgAll, N, F, D>(mut self, name: N, in_args: IA::strs, out_args: OA::strs, f: F) -> Self
    where N: Into<MemberName<'static>>, F: MakeHandler<<H as Handlers>::Method, IA, OA, D> {
        let f = f.make();

        let m = MethodInfo { name: name.into(), handler: DebugMethod(f), 
            i_args: build_argvec::<IA>(in_args), o_args: build_argvec::<OA>(out_args), anns: Default::default() };
        self.info.methods.push(m);
        self.last = Some(MetSigProp::Method);
        self
    }

}

impl<'a, I: 'static> IfaceInfoBuilder<'a, I, Par> {
    pub fn prop_rw<T, N, G, S>(mut self, name: N, getf: G, setf: S) -> Self
    where T: Arg + Append + for<'z> Get<'z> + Send + Sync + 'static,
    N: Into<MemberName<'static>>,
    G: Fn(&I, &ParInfo) -> Result<T, MethodErr> + Send + Sync + 'static,
    S: Fn(&I, &ParInfo, T) -> Result<(), MethodErr> + Send + Sync + 'static
    {
        let p = PropInfo::new(name.into(), T::signature(), Some(Par::typed_getprop(getf)), Some(Par::typed_setprop(setf)));
        self.info.props.push(p);
        self.last = Some(MetSigProp::Prop);
        self
    }

    pub fn prop_ro<T, N, G>(mut self, name: N, getf: G) -> Self
    where T: Arg + Append + Send + Sync + 'static,
    N: Into<MemberName<'static>>,
    G: Fn(&I, &ParInfo) -> Result<T, MethodErr> + Send + Sync + 'static,
    {
        let p = PropInfo::new(name.into(), T::signature(), Some(Par::typed_getprop(getf)), None);
        self.info.props.push(p);
        self.last = Some(MetSigProp::Prop);
        self
    }

}

impl<H: Handlers> MethodInfo<'_, H> {
    pub fn new(name: MemberName<'static>, f: H::Method) -> Self {
        MethodInfo { name: name, handler: DebugMethod(f),
            i_args: Default::default(), o_args: Default::default(), anns: Default::default() }
    }
}

impl<H: Handlers> PropInfo<'_, H> {
    pub fn new(name: MemberName<'static>, sig: Signature<'static>, get: Option<H::GetProp>, 
        set: Option<H::SetProp>) -> Self {
        let a = match (&get, &set) {
            (Some(_), Some(_)) => Access::ReadWrite,
            (Some(_), None) => Access::Read,
            (None, Some(_)) => Access::Write,
            _ => unimplemented!(),
        };
        PropInfo { name, handlers: DebugProp(get, set), sig, auto_emit: true, rw: a, 
            emits: EmitsChangedSignal::True, anns: Default::default() }
    }
}

impl<'a, H: Handlers> IfaceInfo<'a, H> {
    pub fn new_empty(name: IfaceName<'static>) -> Self {
        IfaceInfo { name, methods: vec!(), props: vec!(), signals: vec!(), anns: Default::default(), }
    }

    pub fn new<N, M, P, S>(name: N, methods: M, properties: P, signals: S) -> Self where
        N: Into<IfaceName<'a>>, 
        M: IntoIterator<Item=MethodInfo<'a, H>>, 
        P: IntoIterator<Item=PropInfo<'a, H>>,
        S: IntoIterator<Item=SignalInfo<'a>>
    {
        IfaceInfo {
            name: name.into(),
            methods: methods.into_iter().collect(),
            props: properties.into_iter().collect(),
            signals: signals.into_iter().collect(),
            anns: Default::default(),
        }
    }
}

