"""Krita side of Animator's link: when Animator sends a linked document
again, save any unsaved Krita edits back to it first, then swap the new
version into the same window — frame, active layer, zoom, rotation and
mirror kept. (Krita has no reload of its own, and its Python API can't set
the pan, so the view recenters.)"""

from krita import Extension, Krita
from PyQt5.QtCore import QTimer
from PyQt5.QtGui import QIcon

from . import mailbox

TICK_MS = 400


def reply(path, fields):
    mailbox.write(path + ".to-app", fields)


class AnimatorLink(Extension):
    def __init__(self, parent):
        super().__init__(parent)
        self.sessions = {}
        self.timer = None

    def setup(self):
        self.timer = QTimer(self)
        self.timer.timeout.connect(self.tick)
        self.timer.start(TICK_MS)

    def createActions(self, window):
        pass

    def tick(self):
        for doc in Krita.instance().documents():
            path = doc.fileName()
            if not mailbox.is_linked(path):
                continue
            k = mailbox.key(path)
            msg = mailbox.read(path + ".to-krita")
            session = self.sessions.get(k)
            if session is None:
                self.sessions[k] = mailbox.Session(msg)
                continue
            action = session.next_action(msg, doc.modified())
            if action is None:
                continue
            kind, seq = action
            try:
                if kind == "ready":
                    reply(path, {"seq": seq, "msg": "ready", "saved": 0})
                elif kind == "save":
                    # Synchronous: the file is complete when this returns, so
                    # Animator can read it straight away.
                    if not doc.save():
                        raise RuntimeError("Krita couldn't save the document")
                    reply(path, {"seq": seq, "msg": "ready", "saved": 1})
                elif kind == "reload":
                    self.reload(doc)
                    reply(path, {"seq": seq, "msg": "reloaded"})
            except Exception as e:  # report it rather than leave Animator waiting
                reply(path, {"seq": seq, "msg": "error", "text": str(e)})
            # One document per tick: a reload changes the document list.
            return

    def view_of(self, doc):
        path = mailbox.key(doc.fileName())
        for window in Krita.instance().windows():
            for view in window.views():
                d = view.document()
                if d is not None and mailbox.key(d.fileName()) == path:
                    return window, view
        return Krita.instance().activeWindow(), None

    def reload(self, doc):
        app = Krita.instance()
        path = doc.fileName()
        window, view = self.view_of(doc)
        time = doc.currentTime()
        node = doc.activeNode()
        node_id = node.uniqueId() if node is not None else None
        canvas = view.canvas() if view is not None else None
        pose = (canvas.zoomLevel(), canvas.rotation(), canvas.mirror()) if canvas else None

        new = app.openDocument(path)
        if new is None:
            raise RuntimeError("Krita couldn't reopen the file")
        # The new view first, then close the old document: the window never
        # goes empty.
        new_view = window.addView(new)
        window.showView(new_view)
        new.setCurrentTime(time)
        if node_id is not None:
            n = new.nodeByUniqueID(node_id)
            if n is not None:
                new.setActiveNode(n)
        if pose is not None:
            c = new_view.canvas()
            c.setZoomLevel(pose[0])
            c.setRotation(pose[1])
            c.setMirror(pose[2])
        # Its edits went to Animator before the send; nothing to ask about.
        doc.setModified(False)
        doc.close()
        new_view.showFloatingMessage("Updated from Animator", QIcon(), 2000, 1)
