//! The phone at its own size, scaled to the room there is: it lays its child out at the
//! phone's proportions and scales the whole, so cells, text and strokes keep their ratios.

use std::cell::Cell;

use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene, gsk};

/// No bigger than this on a big window.
const MAX_SCALE: f32 = 1.2;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Fit {
        pub size: Cell<(i32, i32)>,
        pub scale: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Fit {
        const NAME: &'static str = "LinksyDeckFit";
        type Type = super::Fit;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Fit {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Fit {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let (w, h) = self.size.get();
            let n = if orientation == gtk::Orientation::Horizontal { w } else { h };
            (n / 3, n, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some(child) = self.obj().first_child() else { return };
            let (w, h) = self.size.get();
            if w <= 0 || h <= 0 {
                return;
            }
            let scale = (width as f32 / w as f32).min(height as f32 / h as f32).min(MAX_SCALE);
            self.scale.set(scale);
            let at = graphene::Point::new((width as f32 - w as f32 * scale) / 2.0, (height as f32 - h as f32 * scale) / 2.0);
            let _ = child.measure(gtk::Orientation::Horizontal, -1);
            child.allocate(w, h, -1, Some(gsk::Transform::new().translate(&at).scale(scale, scale)));
        }
    }
}

glib::wrapper! {
    pub struct Fit(ObjectSubclass<imp::Fit>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Fit {
    pub fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let fit: Self = glib::Object::new();
        child.set_parent(&fit);
        fit
    }

    /// The phone's own size, which its child is laid out at.
    pub fn set_size(&self, w: i32, h: i32) {
        self.imp().size.set((w, h));
        self.queue_resize();
    }

    pub fn scale(&self) -> f32 {
        self.imp().scale.get()
    }
}
