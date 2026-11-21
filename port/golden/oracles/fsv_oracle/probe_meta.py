from common import Fake
from ticket_meta import ticket_fields
from fsv import formmeta

f = Fake({"GET ticket_form_fields": ticket_fields()})
m = formmeta.load_meta(f, "ticket", "edit", fresh=True, module_id=123)
for c in f.calls:
    print(c)
print(m.rules, m.workspace_id)
