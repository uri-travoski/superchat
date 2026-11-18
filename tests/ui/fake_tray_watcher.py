import gi, sys
gi.require_version('Gio','2.0')
from gi.repository import Gio, GLib
XML='''<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<method name="RegisterStatusNotifierHost"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
<signal name="StatusNotifierItemRegistered"><arg type="s"/></signal>
<signal name="StatusNotifierHostRegistered"/>
</interface></node>'''
items=[]
def call(conn,sender,path,iface,method,params,inv):
    if method=='RegisterStatusNotifierItem':
        s=params.unpack()[0]
        items.append((sender,s))
        open(sys.argv[1],'w').write(sender+' '+s+'\n')
        conn.emit_signal(None,'/StatusNotifierWatcher','org.kde.StatusNotifierWatcher','StatusNotifierItemRegistered',GLib.Variant('(s)',(s,)))
    inv.return_value(None)
def getp(conn,sender,path,iface,prop):
    if prop=='RegisteredStatusNotifierItems': return GLib.Variant('as',[a+b for _,b in items])
    if prop=='IsStatusNotifierHostRegistered': return GLib.Variant('b',True)
    return GLib.Variant('i',0)
def bus(conn,name):
    ni=Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object('/StatusNotifierWatcher',ni.interfaces[0],call,getp,None)
def acq(conn,name): conn.emit_signal(None,'/StatusNotifierWatcher','org.kde.StatusNotifierWatcher','StatusNotifierHostRegistered',None)
Gio.bus_own_name(Gio.BusType.SESSION,'org.kde.StatusNotifierWatcher',Gio.BusNameOwnerFlags.NONE,bus,acq,None)
GLib.MainLoop().run()
