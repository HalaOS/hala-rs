use std::{
    cell::UnsafeCell,
    ops::{self},
};

/// This trait extend [`Deref`](ops::Deref) to add function `deref_map`
pub trait DerefExt: ops::Deref {
    fn deref_map<F, R>(self, f: F) -> MapDeref<Self, F>
    where
        Self: Sized,
        F: Fn(&Self::Target) -> &R,
    {
        MapDeref { deref: self, f }
    }
}

/// The type map [`Deref`](ops::Deref) target to another type.
pub struct MapDeref<T, F> {
    deref: T,
    f: F,
}

impl<T, F, R> ops::Deref for MapDeref<T, F>
where
    F: Fn(&T::Target) -> &R,
    T: ops::Deref,
{
    type Target = R;

    fn deref(&self) -> &Self::Target {
        (self.f)(self.deref.deref())
    }
}

/// Implement [`DerefExt`] for all type that implement trait [`Deref`](ops::Deref)
impl<T> DerefExt for T where T: ops::Deref {}

/// This trait extend [`DerefMut`](ops::DerefMut) to add function `deref_mut_map`
pub trait DerefMutExt: ops::DerefMut {
    fn deref_mut_map<F, R>(self, f: F) -> MapDerefMut<Self, F>
    where
        Self: Sized,
        F: Fn(&mut Self::Target) -> &mut R,
    {
        MapDerefMut {
            deref: UnsafeCell::new(self),
            f,
        }
    }
}

/// The type map [`DerefMut`](ops::DerefMut)'s target to another type.
pub struct MapDerefMut<T, F> {
    deref: UnsafeCell<T>,
    f: F,
}

impl<T, F, R> ops::Deref for MapDerefMut<T, F>
where
    F: Fn(&mut T::Target) -> &mut R,
    T: ops::DerefMut,
{
    type Target = R;

    fn deref(&self) -> &Self::Target {
        // Safety: borrowing checks stil valid for [`MapDerefMut`]
        (self.f)(unsafe { &mut *self.deref.get() })
    }
}

impl<T, F, R> ops::DerefMut for MapDerefMut<T, F>
where
    F: Fn(&mut T::Target) -> &mut R,
    T: ops::DerefMut,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        // Safety: borrowing checks stil valid for [`MapDerefMut`]
        (self.f)(unsafe { &mut *self.deref.get() })
    }
}

/// Implement [`DerefMutExt`] for all type that implement trait [`DerefMut`](ops::DerefMut)
impl<T> DerefMutExt for T where T: ops::DerefMut {}
