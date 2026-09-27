from krita import Krita

from .extension import AnimatorLink

Krita.instance().addExtension(AnimatorLink(Krita.instance()))
