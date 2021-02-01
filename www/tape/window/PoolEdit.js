Ext.define('PBS.TapeManagement.PoolEditWindow', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsPoolEditWindow',
    mixins: ['Proxmox.Mixin.CBind'],

    isCreate: true,
    isAdd: true,
    subject: gettext('Media Pool'),
    cbindData: function(initialConfig) {
	let me = this;

	let poolid = initialConfig.poolid;
	let baseurl = '/api2/extjs/config/media-pool';

	me.isCreate = !poolid;
	me.url = poolid ? `${baseurl}/${encodeURIComponent(poolid)}` : baseurl;
	me.method = poolid ? 'PUT' : 'POST';

	return { };
    },

    items: [
	{
	    fieldLabel: gettext('Name'),
	    name: 'name',
	    xtype: 'pmxDisplayEditField',
	    renderer: Ext.htmlEncode,
	    allowBlank: false,
	    cbind: {
		editable: '{isCreate}',
	    },
	},
	{
	    fieldLabel: gettext('Allocation'),
	    xtype: 'pbsAllocationSelector',
	    name: 'allocation',
	    skipEmptyText: true,
	    allowBlank: true,
	    autoSelect: false,
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
	{
	    fieldLabel: gettext('Retention'),
	    xtype: 'pbsRetentionSelector',
	    name: 'retention',
	    skipEmptyText: true,
	    allowBlank: true,
	    autoSelect: false,
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
    ],
});

