// This file is generated. Do not edit
// @generated

// https://github.com/Manishearth/rust-clippy/issues/702
#![allow(unknown_lints)]
#![allow(clippy::all)]

#![cfg_attr(rustfmt, rustfmt_skip)]

#![allow(box_pointers)]
#![allow(dead_code)]
#![allow(missing_docs)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]
#![allow(trivial_casts)]
#![allow(unsafe_code)]
#![allow(unused_imports)]
#![allow(unused_results)]

const METHOD_DV_EXPORT_KICKOUT_UUID: ::grpcio::Method<super::dv::Kickout, super::dv::Empty> = ::grpcio::Method {
    ty: ::grpcio::MethodType::Unary,
    name: "/DvExport/KickoutUuid",
    req_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
    resp_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
};

const METHOD_DV_EXPORT_UUID_CFG_CHANGED: ::grpcio::Method<super::dv::CfgChangeNotify, super::dv::Empty> = ::grpcio::Method {
    ty: ::grpcio::MethodType::Unary,
    name: "/DvExport/UuidCfgChanged",
    req_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
    resp_mar: ::grpcio::Marshaller { ser: ::grpcio::pb_ser, de: ::grpcio::pb_de },
};

#[derive(Clone)]
pub struct DvExportClient {
    client: ::grpcio::Client,
}

impl DvExportClient {
    pub fn new(channel: ::grpcio::Channel) -> Self {
        DvExportClient {
            client: ::grpcio::Client::new(channel),
        }
    }

    pub fn kickout_uuid_opt(&self, req: &super::dv::Kickout, opt: ::grpcio::CallOption) -> ::grpcio::Result<super::dv::Empty> {
        self.client.unary_call(&METHOD_DV_EXPORT_KICKOUT_UUID, req, opt)
    }

    pub fn kickout_uuid(&self, req: &super::dv::Kickout) -> ::grpcio::Result<super::dv::Empty> {
        self.kickout_uuid_opt(req, ::grpcio::CallOption::default())
    }

    pub fn kickout_uuid_async_opt(&self, req: &super::dv::Kickout, opt: ::grpcio::CallOption) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv::Empty>> {
        self.client.unary_call_async(&METHOD_DV_EXPORT_KICKOUT_UUID, req, opt)
    }

    pub fn kickout_uuid_async(&self, req: &super::dv::Kickout) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv::Empty>> {
        self.kickout_uuid_async_opt(req, ::grpcio::CallOption::default())
    }

    pub fn uuid_cfg_changed_opt(&self, req: &super::dv::CfgChangeNotify, opt: ::grpcio::CallOption) -> ::grpcio::Result<super::dv::Empty> {
        self.client.unary_call(&METHOD_DV_EXPORT_UUID_CFG_CHANGED, req, opt)
    }

    pub fn uuid_cfg_changed(&self, req: &super::dv::CfgChangeNotify) -> ::grpcio::Result<super::dv::Empty> {
        self.uuid_cfg_changed_opt(req, ::grpcio::CallOption::default())
    }

    pub fn uuid_cfg_changed_async_opt(&self, req: &super::dv::CfgChangeNotify, opt: ::grpcio::CallOption) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv::Empty>> {
        self.client.unary_call_async(&METHOD_DV_EXPORT_UUID_CFG_CHANGED, req, opt)
    }

    pub fn uuid_cfg_changed_async(&self, req: &super::dv::CfgChangeNotify) -> ::grpcio::Result<::grpcio::ClientUnaryReceiver<super::dv::Empty>> {
        self.uuid_cfg_changed_async_opt(req, ::grpcio::CallOption::default())
    }
    pub fn spawn<F>(&self, f: F) where F: ::futures::Future<Item = (), Error = ()> + Send + 'static {
        self.client.spawn(f)
    }
}

pub trait DvExport {
    fn kickout_uuid(&mut self, ctx: ::grpcio::RpcContext, req: super::dv::Kickout, sink: ::grpcio::UnarySink<super::dv::Empty>);
    fn uuid_cfg_changed(&mut self, ctx: ::grpcio::RpcContext, req: super::dv::CfgChangeNotify, sink: ::grpcio::UnarySink<super::dv::Empty>);
}

pub fn create_dv_export<S: DvExport + Send + Clone + 'static>(s: S) -> ::grpcio::Service {
    let mut builder = ::grpcio::ServiceBuilder::new();
    let mut instance = s.clone();
    builder = builder.add_unary_handler(&METHOD_DV_EXPORT_KICKOUT_UUID, move |ctx, req, resp| {
        instance.kickout_uuid(ctx, req, resp)
    });
    let mut instance = s;
    builder = builder.add_unary_handler(&METHOD_DV_EXPORT_UUID_CFG_CHANGED, move |ctx, req, resp| {
        instance.uuid_cfg_changed(ctx, req, resp)
    });
    builder.build()
}
